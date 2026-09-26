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


def test_run_native_pack_entries_defaults_level_and_survives_stale_native(tmp_path, monkeypatch, caplog):
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

    # native_function_available() reporting stale/false must not stop the
    # wrapper from refreshing and calling through anyway.
    entries = (ArchiveEntry("Meshes/a.nif", tmp_path / "a.nif", 3),)
    entries[0].source_path.write_bytes(b"nif")
    calls = []

    def fake_pack_archive_entries_2(native_entries, output_path, archive_type, **kwargs):
        calls.append((native_entries, output_path, archive_type, kwargs))
        Path(output_path).write_bytes(b"BA2")
        return len(native_entries)

    monkeypatch.setattr(packer.native_runtime, "native_function_available", lambda name: False)
    monkeypatch.setattr(packer.native_runtime, "pack_archive_entries", fake_pack_archive_entries_2)

    output_path2 = tmp_path / "out2.ba2"
    packer._run_native_pack_entries(entries, str(output_path2), "fo4")

    assert output_path2.read_bytes() == b"BA2"
    assert calls[0][0] == [(str(tmp_path / "a.nif"), "Meshes/a.nif")]

    # native_function_available() reporting the function available logs progress
    # and forwards the jobs kwarg through to the native binding.
    log_entries = (ArchiveEntry("Meshes/b.nif", tmp_path / "b.nif", 3),)
    log_entries[0].source_path.write_bytes(b"nif")
    log_calls = []

    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive_entries",
    )

    def fake_pack_archive_entries_3(native_entries, output_path, archive_type, **kwargs):
        log_calls.append((native_entries, output_path, archive_type, kwargs))
        Path(output_path).write_bytes(b"BA2")
        return len(native_entries)

    monkeypatch.setattr(
        packer.native_runtime,
        "pack_archive_entries",
        fake_pack_archive_entries_3,
    )

    output_path3 = tmp_path / "out3.ba2"
    with caplog.at_level("INFO"):
        packer._run_native_pack_entries(log_entries, str(output_path3), "fo4", jobs=8)

    assert output_path3.read_bytes() == b"BA2"
    assert log_calls[0][0] == [(str(tmp_path / "b.nif"), "Meshes/b.nif")]
    assert log_calls[0][3]["jobs"] == 8
    assert "entries=1" in caplog.text
    assert "jobs=8" in caplog.text


def test_run_native_pack_plans_progress_and_filters(tmp_path, monkeypatch):
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

    # _run_native_pack forwards include/exclude filters straight to the native binding.
    monkeypatch.setattr(packer, "_native_archive_type", lambda *_a, **_k: "fo4")
    filter_calls = []

    def fake_pack_archive(source_dir, output_path, archive_type, **kwargs):
        filter_calls.append((source_dir, output_path, archive_type, kwargs))
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
    filter_output_path = tmp_path / "out_filters.ba2"
    packer._run_native_pack(
        str(source_dir),
        str(filter_output_path),
        "fo4",
        include_prefixes=["Textures/"],
        exclude_prefixes=["Textures/Generated/"],
    )

    assert filter_calls == [
        (
            str(source_dir),
            str(filter_output_path),
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


def test_inventory_data_tree_and_root_strings_entries(tmp_path, monkeypatch):
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

    # _inventory_tree_entries can prefix texture paths under a relative root.
    tree_root = tmp_path / "tree"
    tree_textures_dir = tree_root / "Textures"
    tree_textures_dir.mkdir(parents=True)
    (tree_textures_dir / "test.dds").write_bytes(b"dds")

    tree_entries = packer._inventory_tree_entries(tree_textures_dir, relative_prefix="Textures")

    assert [entry.relative_path for entry in tree_entries] == ["Textures/test.dds"]

    # _inventory_root_strings_entries skips CK-generated dotfile temp orphans.
    strings_root = tmp_path / "strings"
    strings_dir = strings_root / "Strings"
    strings_dir.mkdir(parents=True)
    (strings_dir / "B21_Test_en.STRINGS").write_bytes(b"real")
    (strings_dir / ".B21_Test.esm.02m9o1vv_en.STRINGS").write_bytes(b"orphan")
    (strings_dir / ".B21_Test.esm.ckfix.tmp_en.DLSTRINGS").write_bytes(b"orphan")

    string_entries = packer._inventory_root_strings_entries(strings_dir)

    assert [entry.relative_path for entry in string_entries] == [
        "Strings/B21_Test_en.STRINGS"
    ]


def test_inventory_data_entries_excludes_precombine_sidecars(tmp_path):
    """A precombine .csg/.cdx that ends up in data/ (it belongs loose beside
    the plugin, not in the archived tree) must never be swept into a BA2 —
    the engine only ever reads these two loose next to the plugin."""
    data_dir = tmp_path / "data"
    nested_dir = data_dir / "Meshes" / "PreCombined"
    nested_dir.mkdir(parents=True)
    (data_dir / "B21_Test - Geometry.csg").write_bytes(b"csg")
    (data_dir / "B21_Test.cdx").write_bytes(b"cdx")
    (data_dir / "keep.txt").write_bytes(b"keep")
    (nested_dir / "nested.cdx").write_bytes(b"nested-cdx")
    (nested_dir / "00000800.nif").write_bytes(b"nif")

    entries = packer._inventory_data_entries(data_dir)

    assert sorted(entry.relative_path for entry in entries) == [
        "Meshes/PreCombined/00000800.nif",
        "keep.txt",
    ]


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

    archive_output_dir = tmp_path / "export"
    result = packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=tmp_path,
        archive_output_dir=archive_output_dir,
    )

    assert calls[0]["mod_name"] == mod_name
    assert calls[0]["archive_ext"] == "ba2"
    assert calls[0]["pc"] is True
    assert calls[0]["xbox"] is False
    assert calls[0]["archive_workers"] == 8
    assert calls[0]["fo4_og"] is False
    assert calls[0]["mod_dir"] == str(archive_output_dir)
    assert (archive_output_dir / "B21_Test - Main.ba2").is_file()
    assert not (tmp_path / "mods" / mod_name / "B21_Test - Main.ba2").exists()
    assert result is None
    assert "native inventory progress" in progress_messages


def test_pack_mod_ba2_defaults_to_main_and_texture_labels_pc_and_ps(tmp_path, monkeypatch):
    mod_name = "B21_Test"

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

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fail_run_native_pack_entries)

    # Phase 1: PC defaults to Main/Textures labels, packed straight from the data dir.
    pc_root = tmp_path / "pc"
    _write_mod_file(pc_root, mod_name, "Meshes/test.nif")
    _write_mod_file(pc_root, mod_name, "Textures/test.dds")
    pc_calls = []

    def fake_run_native_pack_pc(source_dir, output_path, game, **kwargs):
        pc_calls.append(
            (
                Path(output_path).name,
                Path(source_dir).name,
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack_pc)

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=pc_root,
    )

    assert pc_calls == [
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

    # Phase 2: PlayStation uses the _ps suffix and gnrl-compatible profile handling.
    def fail_stage_archive_entries_ps(entries, dest_root):
        raise AssertionError("uncapped PlayStation archive packing should not stage files")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries_ps)

    ps_root = tmp_path / "ps"
    _write_mod_file(ps_root, mod_name, "Meshes/test.nif")
    _write_mod_file(ps_root, mod_name, "Textures/test.dds")
    ps_calls = []

    def fake_run_native_pack_ps(source_dir, output_path, game, **kwargs):
        ps_calls.append((Path(output_path).name, kwargs))
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack_ps)

    packer.pack_mod(
        mod_name,
        pc=False,
        ps=True,
        game="fo4",
        project_root=ps_root,
    )

    assert ps_calls == [
        (
            "B21_Test - Main_ps.ba2",
            {"ps": True, "manifest_path": None, "include_prefixes": ["Meshes/"]},
        ),
        (
            "B21_Test - Textures_ps.ba2",
            {
                "texture_archive": True,
                "ps": True,
                "manifest_path": None,
                "include_prefixes": ["Textures/"],
            },
        ),
    ]


def test_pack_mod_ba2_expanded_archives_merges_misc_and_splits_families(tmp_path, monkeypatch):
    mod_name = "B21_Test"

    monkeypatch.setattr(packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2"))
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )

    def fail_stage_archive_entries(entries, dest_root):
        raise AssertionError("eligible archive packing should not stage files")

    monkeypatch.setattr(packer, "_stage_archive_entries", fail_stage_archive_entries)

    calls = []

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                Path(output_path).name,
                [(entry.relative_path, entry.source_path.name) for entry in entries],
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    # Phase 1: root-level scripts + readme merge into a single Misc archive.
    misc_root = tmp_path / "misc"
    _write_mod_file(misc_root, mod_name, "Scripts/test.pex", b"pex")
    _write_mod_file(misc_root, mod_name, "readme.txt", b"readme")

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=misc_root,
        expanded_archives=True,
    )

    assert calls == [
        (
            "B21_Test - Misc.ba2",
            [("Scripts/test.pex", "test.pex"), ("readme.txt", "readme.txt")],
            {"texture_archive": False, "manifest_path": None},
        )
    ]

    # Phase 2: expanded family labels split Meshes and Textures into separate archives.
    calls.clear()
    families_root = tmp_path / "families"
    _write_mod_file(families_root, mod_name, "Meshes/test.nif")
    _write_mod_file(families_root, mod_name, "Textures/test.dds")

    packer.pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        pc_max_res=0,
        pc_effects_max_res=0,
        game="fo4",
        project_root=families_root,
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


def test_pack_mod_direct_entries_fan_out_packs_every_archive_and_propagates_errors(tmp_path, monkeypatch):
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

    # An error from any worker must propagate out of pack_mod, not be swallowed.
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


def test_pack_mod_pc_ba2_root_strings_and_size_validation(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    root_strings_root = tmp_path / "root_strings"
    mod_dir = root_strings_root / "mods" / mod_name
    (mod_dir / "data").mkdir(parents=True)
    strings_file = mod_dir / "Strings" / f"{mod_name}_en.STRINGS"
    strings_file.parent.mkdir(parents=True)
    strings_file.write_bytes(b"strings")
    data_strings_file = mod_dir / "data" / "Strings" / strings_file.name
    data_strings_file.parent.mkdir(parents=True)
    data_strings_file.write_bytes(b"stale")
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
        project_root=root_strings_root,
    )

    assert len(calls) == 1
    assert calls[0][0] == f"{mod_name} - Main.ba2"
    assert calls[0][1] == [(f"Strings/{mod_name}_en.STRINGS", b"strings")]
    assert "include_prefixes" not in calls[0][2]

    # pack_mod validates that the final packed archive stays under the size cap.
    size_cap_root = tmp_path / "size_cap"
    _write_mod_file(size_cap_root, mod_name, "Meshes/a.nif", b"m")

    def fake_run_native_pack_entries_oversized(entries, output_path, game, **kwargs):
        Path(output_path).write_bytes(b"x" * 9001)

    monkeypatch.setattr(packer, "_run_native_pack", fake_run_native_pack_entries_oversized)
    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries_oversized)

    with pytest.raises(RuntimeError, match="exceeding archive max size"):
        packer.pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            pc_max_res=0,
            pc_effects_max_res=0,
            game="fo4",
            project_root=size_cap_root,
            archive_max_bytes=9000,
            expanded_archives=True,
        )


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


def test_prepare_playstation_audio_scenarios(tmp_path, monkeypatch):
    # Phase 0: an xwm with no companion wav is rejected.
    missing_wav_root = tmp_path / "missing_wav"
    xwm_path0 = missing_wav_root / "Sound" / "missing.xwm"
    xwm_path0.parent.mkdir(parents=True)
    xwm_path0.write_bytes(b"xwm")

    with pytest.raises(ValueError, match="companion WAV"):
        packer._prepare_playstation_audio_entries(
            [ArchiveEntry("Sound/missing.xwm", xwm_path0, 3)],
            missing_wav_root / "stage",
        )

    # Phase 1: basic repack - omits the xwm and repacks the fuz with the companion wav.
    phase1_root = tmp_path / "phase1"
    sound_dir = phase1_root / "Sound" / "Voice"
    sound_dir.mkdir(parents=True)
    fuz_path = sound_dir / "line.fuz"
    xwm_path = sound_dir / "line.xwm"
    wav_path = sound_dir / "line.wav"
    mesh_path = phase1_root / "Meshes" / "test.nif"
    mesh_path.parent.mkdir()

    lip_bytes = b"LIP"
    xwm_bytes = b"RIFF\x04\x00\x00\x00XWMA"
    wave_fmt = b"\x01\x00\x01\x00\x44\xac\x00\x00\x88\x58\x01\x00\x02\x00\x10\x00"
    wave_chunks = b"fmt " + len(wave_fmt).to_bytes(4, "little") + wave_fmt
    wav_bytes = b"RIFF" + (len(wave_chunks) + 4).to_bytes(4, "little") + b"WAVE" + wave_chunks
    fuz_path.write_bytes(
        b"FUZE\x01\x00\x00\x00"
        + len(lip_bytes).to_bytes(4, "little")
        + lip_bytes
        + xwm_bytes
    )
    xwm_path.write_bytes(xwm_bytes)
    wav_path.write_bytes(wav_bytes)
    mesh_path.write_bytes(b"mesh")

    entries = [
        ArchiveEntry("Sound/Voice/line.fuz", fuz_path, fuz_path.stat().st_size),
        ArchiveEntry("Sound/Voice/line.xwm", xwm_path, xwm_path.stat().st_size),
        ArchiveEntry("Sound/Voice/line.wav", wav_path, wav_path.stat().st_size),
        ArchiveEntry("Meshes/test.nif", mesh_path, mesh_path.stat().st_size),
    ]

    prepared, adjusted = packer._prepare_playstation_audio_entries(
        entries, phase1_root / "stage"
    )

    assert adjusted is True
    assert [entry.relative_path for entry in prepared] == [
        "Sound/Voice/line.fuz",
        "Sound/Voice/line.wav",
        "Meshes/test.nif",
    ]
    ps_fuz = prepared[0].source_path.read_bytes()
    assert ps_fuz[: 12 + len(lip_bytes)] == fuz_path.read_bytes()[: 12 + len(lip_bytes)]
    assert ps_fuz[12 + len(lip_bytes) :] == wav_bytes

    # Phase 2: convert_to_at9 encodes the wav to at9 and embeds it into the fuz.
    phase2_root = tmp_path / "phase2"
    sound_dir2 = phase2_root / "Sound" / "Voice"
    sound_dir2.mkdir(parents=True)
    fuz_path2 = sound_dir2 / "line.fuz"
    xwm_path2 = sound_dir2 / "line.xwm"
    wav_path2 = sound_dir2 / "line.wav"
    xwm_bytes2 = b"RIFF\x04\x00\x00\x00XWMA"
    wav_path2.write_bytes(b"RIFF\x04\x00\x00\x00WAVE")
    xwm_path2.write_bytes(xwm_bytes2)
    fuz_path2.write_bytes(
        b"FUZE\x01\x00\x00\x00"
        + len(lip_bytes).to_bytes(4, "little")
        + lip_bytes
        + xwm_bytes2
    )
    at9_fmt = (
        b"\xfe\xff"
        + bytes(22)
        + bytes.fromhex("d242e147ba368d4d88fc61654f8c836c")
    )
    at9_bytes = (
        b"RIFF"
        + (len(at9_fmt) + 12).to_bytes(4, "little")
        + b"WAVEfmt "
        + len(at9_fmt).to_bytes(4, "little")
        + at9_fmt
    )
    encode_calls = []

    def encode_at9(source, output):
        encode_calls.append(Path(source).name)
        output_path = Path(output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_bytes(at9_bytes)

    monkeypatch.setattr(packer.audio_native_runtime, "encode_at9", encode_at9)
    entries2 = [
        ArchiveEntry("Sound/Voice/line.fuz", fuz_path2, fuz_path2.stat().st_size),
        ArchiveEntry("Sound/Voice/line.xwm", xwm_path2, xwm_path2.stat().st_size),
        ArchiveEntry("Sound/Voice/line.wav", wav_path2, wav_path2.stat().st_size),
    ]

    prepared2, adjusted2 = packer._prepare_playstation_audio_entries(
        entries2,
        phase2_root / "stage",
        convert_to_at9=True,
    )

    assert adjusted2 is True
    assert [entry.relative_path for entry in prepared2] == [
        "Sound/Voice/line.fuz",
        "Sound/Voice/line.at9",
    ]
    assert encode_calls == ["line.wav"]
    assert prepared2[0].source_path.read_bytes()[12 + len(lip_bytes) :] == at9_bytes
    assert prepared2[1].source_path.read_bytes() == at9_bytes

    # Phase 3: an existing at9-backed fuz without a wav is preserved unchanged.
    phase3_root = tmp_path / "phase3"
    phase3_root.mkdir()
    fuz_path3 = phase3_root / "line.fuz"
    fuz_path3.write_bytes(b"FUZE\x01\0\0\0\0\0\0\0" + at9_bytes)

    prepared3, adjusted3 = packer._prepare_playstation_audio_entries(
        [ArchiveEntry("Sound/line.fuz", fuz_path3, fuz_path3.stat().st_size)],
        phase3_root / "stage",
        convert_to_at9=True,
    )

    assert adjusted3 is False
    assert prepared3 == [ArchiveEntry("Sound/line.fuz", fuz_path3, fuz_path3.stat().st_size)]


def test_pack_mod_playstation_rewrites_audio_entries_before_native_pack(tmp_path, monkeypatch):
    mod_name = "B21_Test"
    voice_root = "Sound/Voice/B21_Test.esp/MaleTest/00000001_1"
    lip_bytes = b"LIP"
    xwm_bytes = b"RIFF\x04\x00\x00\x00XWMA"
    wave_fmt = b"\x01\x00\x01\x00\x44\xac\x00\x00\x88\x58\x01\x00\x02\x00\x10\x00"
    wave_chunks = b"fmt " + len(wave_fmt).to_bytes(4, "little") + wave_fmt
    wav_bytes = b"RIFF" + (len(wave_chunks) + 4).to_bytes(4, "little") + b"WAVE" + wave_chunks
    fuz_bytes = (
        b"FUZE\x01\x00\x00\x00"
        + len(lip_bytes).to_bytes(4, "little")
        + lip_bytes
        + xwm_bytes
    )
    _write_mod_file(tmp_path, mod_name, voice_root + ".fuz", fuz_bytes)
    _write_mod_file(tmp_path, mod_name, voice_root + ".xwm", xwm_bytes)
    _write_mod_file(tmp_path, mod_name, voice_root + ".wav", wav_bytes)
    calls = []

    monkeypatch.setattr(
        packer, "get_profile", lambda game: SimpleNamespace(archive_format="ba2")
    )
    monkeypatch.setattr(
        packer.native_runtime,
        "native_function_available",
        lambda name: name == "pack_archive",
    )
    monkeypatch.setattr(
        packer,
        "_run_native_pack",
        lambda *args, **kwargs: (_ for _ in ()).throw(
            AssertionError("adjusted PlayStation audio must use planned entries")
        ),
    )

    def fake_run_native_pack_entries(entries, output_path, game, **kwargs):
        calls.append(
            (
                [entry.relative_path for entry in entries],
                entries[0].source_path.read_bytes(),
                Path(output_path).name,
                kwargs,
            )
        )
        Path(output_path).write_bytes(b"BA2")

    monkeypatch.setattr(packer, "_run_native_pack_entries", fake_run_native_pack_entries)

    packer.pack_mod(
        mod_name,
        pc=False,
        ps=True,
        game="fo4",
        project_root=tmp_path,
    )

    entry_paths, rewritten_fuz, output_name, kwargs = calls[0]
    assert output_name == "B21_Test - Main_ps.ba2"
    assert kwargs["ps"] is True
    assert entry_paths == [
        voice_root + ".fuz",
        voice_root + ".wav",
    ]
    assert rewritten_fuz[12 + len(lip_bytes) :] == wav_bytes
