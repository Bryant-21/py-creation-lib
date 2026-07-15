from __future__ import annotations

import os
from pathlib import Path
from types import SimpleNamespace

import pytest

from creation_lib.build import packer
from creation_lib.build.archive_plan import ArchiveEntry


def _write_mod_file(
    project_root: Path,
    mod_name: str,
    rel_path: str,
    content: bytes = b"x",
) -> None:
    path = project_root / "mods" / mod_name / "data" / Path(rel_path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)


def test_run_native_pack_entries_defaults_level_to_none(tmp_path, monkeypatch):
    captured = {}
    out = tmp_path / "out.ba2"

    def fake_pack_archive_entries(*args, **kwargs):
        captured.update(kwargs)
        out.write_bytes(b"BA2")
        return 1

    monkeypatch.setattr(
        packer.native_runtime, "native_function_available", lambda name: True
    )
    monkeypatch.setattr(
        packer.native_runtime, "pack_archive_entries", fake_pack_archive_entries
    )

    packer._run_native_pack_entries(
        (ArchiveEntry("Textures/a.dds", tmp_path / "a.dds", 10),),
        str(out),
        "fo4",
        texture_archive=True,
    )
    assert captured["compression_level"] is None


def test_run_native_pack_plans_forwards_structured_progress(tmp_path, monkeypatch):
    entry = ArchiveEntry("Meshes/a.nif", tmp_path / "a.nif", 3)
    planned = SimpleNamespace(texture_archive=False, entries=(entry,))
    output = tmp_path / "out.ba2"
    observed = []
    native_returns = []

    monkeypatch.setattr(packer, "_native_archive_type", lambda *_a, **_k: "gnrl")

    def fake_pack_archive_plans(plans, *, total_workers, progress):
        assert plans == [
            (
                str(output),
                "gnrl",
                False,
                [(str(entry.source_path), entry.relative_path, entry.size)],
            )
        ]
        assert total_workers == 6
        event = {
            "phase": "pack",
            "message": "Archive packed native: name=out.ba2",
            "completed": 1,
            "total": 1,
        }
        native_returns.append(progress(event))
        return 1

    monkeypatch.setattr(
        packer.native_runtime,
        "pack_archive_plans",
        fake_pack_archive_plans,
    )

    assert packer._run_native_pack_plans(
        [(planned, output)],
        "fo4",
        total_workers=6,
        progress=lambda event: observed.append(event),
    ) == 1
    assert observed == [
        {
            "phase": "pack",
            "message": "Archive packed native: name=out.ba2",
            "completed": 1,
            "total": 1,
        }
    ]
    assert native_returns == [True]


def test_run_native_pack_passes_filters_to_native_binding(tmp_path, monkeypatch):
    calls = []

    def fake_pack_archive(source_dir, output_path, archive_type, **kwargs):
        calls.append((source_dir, output_path, archive_type, kwargs))
        Path(output_path).write_bytes(b"BA2")
        return 1

    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )
    monkeypatch.setattr(
        packer.native_runtime,
        "_require_native_function",
        lambda name: fake_pack_archive,
    )

    source_dir = tmp_path / "data"
    source_dir.mkdir()
    output_path = tmp_path / "out.ba2"
    packer._run_native_pack(
        str(source_dir),
        str(output_path),
        "fo4",
        include_prefixes=["Textures/"],
        exclude_prefixes=["Textures/Generated/"],
    )

    assert calls == [
        (
            str(source_dir),
            str(output_path),
            "fo4",
            {
                "compress": True,
                "compression_level": None,
                "share_data": False,
                "manifest_path": None,
                "jobs": 0,
                "include_prefixes": ["Textures/"],
                "exclude_prefixes": ["Textures/Generated/"],
            },
        )
    ]


def test_run_native_pack_entries_logs_and_writes_archive(tmp_path, monkeypatch, caplog):
    entries = (
        ArchiveEntry(
            "Meshes/a.nif",
            tmp_path / "a.nif",
            3,
        ),
    )
    entries[0].source_path.write_bytes(b"nif")
    calls = []

    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive_entries",
    )

    def fake_pack_archive_entries(native_entries, output_path, archive_type, **kwargs):
        calls.append((native_entries, output_path, archive_type, kwargs))
        Path(output_path).write_bytes(b"BA2")
        return len(native_entries)

    monkeypatch.setattr(
        packer.native_runtime,
        "pack_archive_entries",
        fake_pack_archive_entries,
    )

    output_path = tmp_path / "out.ba2"
    with caplog.at_level("INFO"):
        packer._run_native_pack_entries(entries, str(output_path), "fo4")

    assert output_path.read_bytes() == b"BA2"
    assert calls[0][0] == [(str(tmp_path / "a.nif"), "Meshes/a.nif")]
    assert "entries=1" in caplog.text


def test_run_native_pack_entries_passes_jobs(tmp_path, monkeypatch, caplog):
    entries = (
        ArchiveEntry(
            "Textures/a.dds",
            tmp_path / "a.dds",
            3,
        ),
    )
    entries[0].source_path.write_bytes(b"dds")
    calls = []

    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive_entries",
    )

    def fake_pack_archive_entries(native_entries, output_path, archive_type, **kwargs):
        calls.append(kwargs)
        Path(output_path).write_bytes(b"BA2")
        return len(native_entries)

    monkeypatch.setattr(
        packer.native_runtime,
        "pack_archive_entries",
        fake_pack_archive_entries,
    )

    output_path = tmp_path / "out.ba2"
    with caplog.at_level("INFO"):
        packer._run_native_pack_entries(
            entries,
            str(output_path),
            "fo4",
            texture_archive=True,
            jobs=8,
        )

    assert calls[0]["jobs"] == 8
    assert "jobs=8" in caplog.text


def test_run_native_pack_entries_lets_wrapper_refresh_stale_native(tmp_path, monkeypatch):
    entries = (
        ArchiveEntry(
            "Meshes/a.nif",
            tmp_path / "a.nif",
            3,
        ),
    )
    entries[0].source_path.write_bytes(b"nif")
    calls = []

    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: False,
    )

    def fake_pack_archive_entries(native_entries, output_path, archive_type, **kwargs):
        calls.append((native_entries, output_path, archive_type, kwargs))
        Path(output_path).write_bytes(b"BA2")
        return len(native_entries)

    monkeypatch.setattr(
        packer.native_runtime,
        "pack_archive_entries",
        fake_pack_archive_entries,
    )

    output_path = tmp_path / "out.ba2"
    packer._run_native_pack_entries(entries, str(output_path), "fo4")

    assert output_path.read_bytes() == b"BA2"
    assert calls[0][0] == [(str(tmp_path / "a.nif"), "Meshes/a.nif")]


def test_inventory_data_entries_excludes_texture_tree_without_traversing_it(tmp_path, monkeypatch):
    data_dir = tmp_path / "data"
    textures_dir = data_dir / "Textures"
    meshes_dir = data_dir / "Meshes"
    textures_dir.mkdir(parents=True)
    meshes_dir.mkdir()
    (textures_dir / "skip.dds").write_bytes(b"dds")
    (meshes_dir / "keep.nif").write_bytes(b"nif")

    original_rglob = Path.rglob

    def tracking_rglob(self, pattern):
        if self == textures_dir:
            raise AssertionError("non-texture inventory descended into Textures")
        return original_rglob(self, pattern)

    monkeypatch.setattr(Path, "rglob", tracking_rglob)

    entries = packer._inventory_data_entries(data_dir, include_textures=False)

    assert [entry.relative_path for entry in entries] == ["Meshes/keep.nif"]


def test_inventory_tree_entries_can_prefix_texture_paths(tmp_path):
    textures_dir = tmp_path / "Textures"
    textures_dir.mkdir()
    (textures_dir / "test.dds").write_bytes(b"dds")

    entries = packer._inventory_tree_entries(textures_dir, relative_prefix="Textures")

    assert [entry.relative_path for entry in entries] == ["Textures/test.dds"]


def test_pack_mod_prefers_native_mod_archive_packer(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif")
    calls: list[dict] = []
    progress_messages: list[str] = []

    monkeypatch.setattr(packer.os, "cpu_count", lambda: 16)
    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name in {"pack_archive", "pack_mod_archives"},
    )

    def fail_legacy_pack(*args, **kwargs):
        raise AssertionError("native mod archive path should bypass legacy pack helpers")

    def fake_pack_mod_archives(config, progress=None):
        calls.append(config)
        if progress:
            progress({"message": "native inventory progress"})
        archive_path = Path(config["mod_dir"]) / "B21_Test - Main.ba2"
        archive_path.write_bytes(b"BA2")
        return {
            "archives": [
                {
                    "platform": "pc",
                    "name": archive_path.name,
                    "file_count": 1,
                    "bytes": 3,
                    "elapsed_secs": 0.01,
                }
            ],
            "inventory_elapsed_secs": 0.01,
            "planning_elapsed_secs": 0.01,
        }

    monkeypatch.setattr(packer, "_run_native_pack", fail_legacy_pack)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fail_legacy_pack)
    monkeypatch.setattr(packer.native_runtime, "pack_mod_archives", fake_pack_mod_archives)
    monkeypatch.setattr(packer._log, "info", lambda msg, *args: progress_messages.append(msg % args if args else msg))

    result = packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
    )

    assert calls[0]["mod_name"] == mod_name
    assert calls[0]["archive_ext"] == "ba2"
    assert calls[0]["pc"] is True
    assert calls[0]["xbox"] is False
    assert calls[0]["archive_workers"] == 8
    assert result is None
    assert "native inventory progress" in progress_messages


def test_pack_mod_fo4_ba2_defaults_to_main_and_texture_labels(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif")
    _write_mod_file(tmp_path, mod_name, "Textures/test.dds")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    def fail_run_native_pack_entries(entries, output_path, game, **kwargs):
        raise AssertionError("compact PC BA2 packing should use direct archive filters")

    def fake_run_native_pack(source_dir, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                Path(source_dir).name,
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fail_run_native_pack_entries)
    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
    )

    assert calls == [
        (
            "B21_Test - Main.ba2",
            "data",
            {"manifest_path": None, "include_prefixes": ["Meshes/"]},
        ),
        (
            "B21_Test - Textures.ba2",
            "data",
            {"texture_archive": True, "manifest_path": None, "include_prefixes": ["Textures/"]},
        ),
    ]


def test_pack_mod_ba2_can_opt_into_expanded_family_labels(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif")
    _write_mod_file(tmp_path, mod_name, "Textures/test.dds")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        expanded_archives=True,
    )

    assert calls == [
        (
            "B21_Test - Meshes.ba2",
            [("Meshes/test.nif", "test.nif")],
            {"texture_archive": False, "manifest_path": None},
        ),
        (
            "B21_Test - Textures.ba2",
            [("Textures/test.dds", "test.dds")],
            {"texture_archive": True, "manifest_path": None},
        ),
    ]


def test_pack_mod_direct_entries_fan_out_packs_every_archive(tmp_path, monkeypatch):
    import threading

    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif")
    _write_mod_file(tmp_path, mod_name, "Materials/test.bgsm")
    _write_mod_file(tmp_path, mod_name, "Textures/test.dds")
    lock = threading.Lock()
    packed: list[str] = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("direct-entries packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        with lock:
            packed.append(Path(output_path).name)
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        expanded_archives=True,
        archive_workers=4,
    )

    # Order is nondeterministic under the thread pool; every planned archive must
    # still be packed exactly once.
    assert sorted(packed) == [
        "B21_Test - Materials.ba2",
        "B21_Test - Meshes.ba2",
        "B21_Test - Textures.ba2",
    ]


def test_pack_mod_direct_entries_fan_out_propagates_errors(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif")
    _write_mod_file(tmp_path, mod_name, "Textures/test.dds")

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def boom(entries, output_path, game, **kwargs):
        raise RuntimeError("pack failed")

    monkeypatch.setattr(packer, "_run_native_pack_entries", boom)

    with pytest.raises(RuntimeError, match="pack failed"):
        packer.pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            pc_max_res=0,
            pc_effects_max_res=0,
            game="fo4",
            project_root=tmp_path,
            expanded_archives=True,
            archive_workers=4,
        )


def test_pack_mod_pc_ba2_packs_root_data_files_into_main_archive(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/test.nif", b"nif")
    _write_mod_file(tmp_path, mod_name, "readme.txt", b"readme")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
    )

    assert calls == [
        (
            "B21_Test - Main.ba2",
            [("Meshes/test.nif", "test.nif"), ("readme.txt", "readme.txt")],
            {"texture_archive": False, "manifest_path": None},
        ),
    ]


def test_pack_mod_fo4_ba2_merges_root_data_into_misc_when_scripts_exist(
    tmp_path,
    monkeypatch,
):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Scripts/test.pex", b"pex")
    _write_mod_file(tmp_path, mod_name, "readme.txt", b"readme")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        expanded_archives=True,
    )

    assert calls == [
        (
            "B21_Test - Misc.ba2",
            [("Scripts/test.pex", "test.pex"), ("readme.txt", "readme.txt")],
            {"texture_archive": False, "manifest_path": None},
        )
    ]


def test_pack_mod_pc_ba2_packs_root_strings_into_main_archive(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    (mod_dir / "data").mkdir(parents=True)
    strings_file = mod_dir / "Strings" / f"{mod_name}_en.STRINGS"
    strings_file.parent.mkdir(parents=True)
    strings_file.write_bytes(b"strings")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.read_bytes()) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
    )

    assert len(calls) == 1
    assert calls[0][0] == f"{mod_name} - Main.ba2"
    assert calls[0][1] == [(f"Strings/{mod_name}_en.STRINGS", b"strings")]
    assert "include_prefixes" not in calls[0][2]


def test_pack_mod_pc_ba2_with_resize_uses_texture_stage(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    data_dir = tmp_path / "mods" / mod_name / "data"
    _write_mod_file(tmp_path, mod_name, "Textures/test.dds")
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fake_prepare_texture_root(texture_src_dir, dest_root, max_res, effects_max_res):
        dest_texture_dir = os.path.join(dest_root, "Textures")
        os.makedirs(dest_texture_dir, exist_ok=True)
        Path(dest_texture_dir, "test.dds").write_bytes(b"dds")

    def fake_run_native_pack(source_dir, output_path, game, **kwargs):
        calls.append((source_dir, output_path, game, kwargs))
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_prepare_texture_root", fake_prepare_texture_root)
    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=512,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
    )

    assert len(calls) == 1
    assert calls[0][0] != str(data_dir)
    assert calls[0][0].endswith(os.path.join("_deploy_tmp", "planned_textures"))
    assert "include_prefixes" not in calls[0][3]
    assert "exclude_prefixes" not in calls[0][3]


def test_pack_mod_splits_oversized_main_into_category_archives(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/a.nif", b"m" * 4000)
    _write_mod_file(tmp_path, mod_name, "Scripts/a.pex", b"p" * 4000)
    _write_mod_file(tmp_path, mod_name, "Sound/a.xwm", b"s" * 4000)
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name in {"pack_archive", "pack_archive_entries"},
    )
    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("split archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        archive_max_bytes=9000,
    )

    assert [call[0] for call in calls] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Sounds.ba2",
        "B21_Test - Scripts.ba2",
    ]
    assert calls[0][1] == [("Meshes/a.nif", "a.nif")]
    assert calls[1][1] == [("Sound/a.xwm", "a.xwm")]
    assert calls[2][1] == [("Scripts/a.pex", "a.pex")]
    assert calls[0][2]["texture_archive"] is False
    assert calls[1][2]["texture_archive"] is False
    assert calls[2][2]["texture_archive"] is False


def test_pack_mod_shards_oversized_textures(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Textures/a.dds", b"a" * 4000)
    _write_mod_file(tmp_path, mod_name, "Textures/b.dds", b"b" * 4000)
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name in {"pack_archive", "pack_archive_entries"},
    )
    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("split archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        archive_max_bytes=9000,
    )

    assert [call[0] for call in calls] == [
        "B21_Test - Textures1.ba2",
        "B21_Test - Textures2.ba2",
    ]
    assert calls[0][1] == [("Textures/a.dds", "a.dds")]
    assert calls[1][1] == [("Textures/b.dds", "b.dds")]
    assert all(call[2]["texture_archive"] is True for call in calls)


def test_pack_mod_splits_root_strings_into_main_archive(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    _write_mod_file(tmp_path, mod_name, "Meshes/a.nif", b"m" * 4000)
    strings_file = mod_dir / "Strings" / f"{mod_name}_en.STRINGS"
    strings_file.parent.mkdir(parents=True)
    strings_file.write_bytes(b"s" * 4000)
    calls = []

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name in {"pack_archive", "pack_archive_entries"},
    )
    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("split archive packing should not stage files")

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        archive_max_bytes=9000,
    )

    assert [call[0] for call in calls] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Main.ba2",
    ]
    assert calls[1][1] == [(f"Strings/{mod_name}_en.STRINGS", f"{mod_name}_en.STRINGS")]


def test_pack_mod_validates_final_packed_size(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    _write_mod_file(tmp_path, mod_name, "Meshes/a.nif", b"m")

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        Path(output_path).write_bytes(b"x" * 9001)

    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    with pytest.raises(RuntimeError, match="exceeding archive max size"):
        packer.pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            pc_max_res=0,
            pc_effects_max_res=0,
            game="fo4",
            project_root=tmp_path,
            archive_max_bytes=9000,
        )
