from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.build.archive_plan import (
    DEFAULT_ARCHIVE_MAX_BYTES,
    ArchiveEntry,
    classify_archive_family,
    discover_mod_archives,
    gib_to_bytes,
    plan_archive_outputs,
)


def _entry(rel_path: str, size: int, root: Path) -> ArchiveEntry:
    path = root / rel_path
    return ArchiveEntry(rel_path, path, size)


def test_default_archive_max_bytes_is_16_gib():
    assert DEFAULT_ARCHIVE_MAX_BYTES == 16 * 1024**3
    assert gib_to_bytes(16.0) == DEFAULT_ARCHIVE_MAX_BYTES


def test_gib_to_bytes_rejects_non_positive_values():
    with pytest.raises(ValueError):
        gib_to_bytes(0)
    with pytest.raises(ValueError):
        gib_to_bytes(-1)


@pytest.mark.parametrize(
    ("path", "family"),
    [
        ("Textures/foo.dds", "Textures"),
        ("data/Textures/foo.dds", "Textures"),
        ("Interface/menu.swf", "Interface"),
        ("data/Interface/menu.swf", "Interface"),
        ("Materials/foo.bgsm", "Materials"),
        ("Meshes/foo/material.bgem", "Materials"),
        ("Strings/B21_en.STRINGS", "Strings"),
        ("data/foo.dlstrings", "Strings"),
        ("Sound/fx/foo.xwm", "Sounds"),
        ("Music/theme.wav", "Sounds"),
        ("Meshes/Actors/Anim.hkx", "Animations"),
        ("Meshes/Actors/Animations/idle.txt", "Animations"),
        ("Meshes/foo.nif", "Meshes"),
        ("Meshes/AnimTextData/AnimationEventInfo/123.txt", "Meshes"),
        ("Scripts/foo.pex", "Scripts"),
        ("data/Scripts/foo.pex", "Scripts"),
    ],
)
def test_classify_archive_family(path: str, family: str):
    assert classify_archive_family(path) == family


def test_classify_archive_family_routes_lod_meshes_to_lod():
    assert classify_archive_family("Meshes/Terrain/Appalachia/Foo.bto") == "LOD"
    assert classify_archive_family("data/Meshes/Terrain/Appalachia/Foo.BTO") == "LOD"
    assert classify_archive_family("Meshes/Terrain/Appalachia/Foo.btr") == "LOD"


def test_plan_archive_outputs_preserves_small_names(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/test.nif", 10, tmp_path),
            _entry("Scripts/test.pex", 10, tmp_path),
            _entry("Textures/test.dds", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Main.ba2",
        "B21_Test - Textures.ba2",
    ]
    assert [plan.label for plan in plans] == ["Main", "Textures"]
    assert [plan.texture_archive for plan in plans] == [False, True]
    assert [entry.relative_path for entry in plans[0].entries] == [
        "Meshes/test.nif",
        "Scripts/test.pex",
    ]


def test_plan_archive_outputs_uses_expanded_fo4_labels(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/test.nif", 10, tmp_path),
            _entry("Scripts/test.pex", 10, tmp_path),
            _entry("readme.txt", 10, tmp_path),
            _entry("Textures/test.dds", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Misc.ba2",
        "B21_Test - Textures.ba2",
    ]
    assert [plan.family for plan in plans] == ["Meshes", "Scripts", "Textures"]
    assert [plan.label for plan in plans] == ["Meshes", "Misc", "Textures"]
    assert [entry.relative_path for entry in plans[1].entries] == [
        "Scripts/test.pex",
        "readme.txt",
    ]


def test_plan_archive_outputs_uses_ba2_estimate_for_compressible_lod_archives(
    tmp_path: Path,
):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry(f"Meshes/Terrain/Appalachia/{idx:02}.bto", 30_000, tmp_path)
            for idx in range(12)
        ],
        "ba2",
        "",
        100_000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - LOD1.ba2",
        "B21_Test - LOD2.ba2",
        "B21_Test - LOD3.ba2",
    ]
    assert [len(plan.entries) for plan in plans] == [4, 4, 4]


def test_plan_archive_outputs_uses_ba2_estimate_for_texture_archives(
    tmp_path: Path,
):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Textures/a.dds", 5000, tmp_path),
            _entry("Textures/b.dds", 5000, tmp_path),
        ],
        "ba2",
        "",
        14_000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == ["B21_Test - Textures.ba2"]
    assert [len(plan.entries) for plan in plans] == [2]


def test_plan_archive_outputs_splits_lod_and_terrain_by_archive_type(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/Objects/a.bto", 10, tmp_path),
            _entry("Textures/Terrain/Appalachia/Appalachia.4.0.0.dds", 10, tmp_path),
            _entry("Materials/Terrain/Appalachia/blend.bgsm", 10, tmp_path),
            _entry("Textures/Terrain/Appalachia/lswamprocks01_d.dds", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
        game="fo4",
        expanded_archives=True,
    )

    by_label = {plan.label: plan for plan in plans}
    assert [entry.relative_path for entry in by_label["LOD"].entries] == [
        "Meshes/Terrain/Appalachia/Objects/a.bto"
    ]
    assert by_label["LOD"].texture_archive is False
    assert [entry.relative_path for entry in by_label["LODTextures"].entries] == [
        "Textures/Terrain/Appalachia/Appalachia.4.0.0.dds"
    ]
    assert by_label["LODTextures"].texture_archive is True
    assert [entry.relative_path for entry in by_label["Terrain"].entries] == [
        "Materials/Terrain/Appalachia/blend.bgsm"
    ]
    assert by_label["Terrain"].texture_archive is False
    assert [entry.relative_path for entry in by_label["TerrainTextures"].entries] == [
        "Textures/Terrain/Appalachia/lswamprocks01_d.dds"
    ]
    assert by_label["TerrainTextures"].texture_archive is True


def test_plan_archive_outputs_shards_sounds_without_relying_on_compression(
    tmp_path: Path,
):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Sound/a.fuz", 5000, tmp_path),
            _entry("Sound/b.fuz", 5000, tmp_path),
        ],
        "ba2",
        "",
        14_000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Sounds1.ba2",
        "B21_Test - Sounds2.ba2",
    ]
    assert [len(plan.entries) for plan in plans] == [1, 1]


def test_plan_archive_outputs_defaults_fo4_to_compact_labels(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/test.nif", 10, tmp_path),
            _entry("Scripts/test.pex", 10, tmp_path),
            _entry("Textures/test.dds", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
        game="fo4",
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Main.ba2",
        "B21_Test - Textures.ba2",
    ]
    assert [entry.relative_path for entry in plans[0].entries] == [
        "Meshes/test.nif",
        "Scripts/test.pex",
    ]


def test_plan_archive_outputs_expanded_archives_apply_to_non_fo4_games(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/test.nif", 10, tmp_path),
            _entry("Scripts/test.pex", 10, tmp_path),
            _entry("Textures/test.dds", 10, tmp_path),
        ],
        "bsa",
        "",
        1024 * 1024,
        game="skyrimse",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.bsa",
        "B21_Test - Scripts.bsa",
        "B21_Test - Textures.bsa",
    ]


def test_plan_archive_outputs_uses_fo4_meshes_extra_for_second_mesh_archive(
    tmp_path: Path,
):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Meshes/b.nif", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - MeshesExtra.ba2",
    ]
    assert [plan.label for plan in plans] == ["Meshes", "MeshesExtra"]
    assert [entry.relative_path for entry in plans[0].entries] == ["Meshes/a.nif"]
    assert [entry.relative_path for entry in plans[1].entries] == ["Meshes/b.nif"]


def test_plan_archive_outputs_numbers_additional_fo4_mesh_archives(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Meshes/b.nif", 4000, tmp_path),
            _entry("Meshes/c.nif", 4000, tmp_path),
            _entry("Meshes/d.nif", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - MeshesExtra.ba2",
        "B21_Test - MeshesExtra1.ba2",
        "B21_Test - MeshesExtra2.ba2",
    ]
    assert [plan.label for plan in plans] == [
        "Meshes",
        "MeshesExtra",
        "MeshesExtra1",
        "MeshesExtra2",
    ]


def test_plan_archive_outputs_numbers_additional_fo4_misc_archives(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Scripts/a.pex", 4000, tmp_path),
            _entry("Scripts/b.pex", 4000, tmp_path),
            _entry("Scripts/c.pex", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
        game="fo4",
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Misc.ba2",
        "B21_Test - Misc1.ba2",
        "B21_Test - Misc2.ba2",
    ]
    assert [plan.label for plan in plans] == ["Misc", "Misc1", "Misc2"]


def test_plan_archive_outputs_keeps_lod_in_main_when_small(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/a.bto", 10, tmp_path),
            _entry("Scripts/a.pex", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
    )

    assert [plan.output_name for plan in plans] == ["B21_Test - Main.ba2"]
    assert [entry.relative_path for entry in plans[0].entries] == [
        "Meshes/Terrain/Appalachia/a.bto",
        "Scripts/a.pex",
    ]


def test_plan_archive_outputs_sorts_entries_deterministically(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Textures/c.dds", 10, tmp_path),
            _entry("Meshes/b.nif", 10, tmp_path),
            _entry("Meshes/a.nif", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
    )

    main_plan = plans[0]
    texture_plan = plans[1]
    assert [entry.relative_path for entry in main_plan.entries] == [
        "Meshes/a.nif",
        "Meshes/b.nif",
    ]
    assert [entry.relative_path for entry in texture_plan.entries] == ["Textures/c.dds"]


def test_plan_archive_outputs_splits_oversized_main_by_category(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Scripts/a.pex", 4000, tmp_path),
            _entry("Sound/a.xwm", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Sounds.ba2",
        "B21_Test - Scripts.ba2",
    ]


def test_plan_archive_outputs_splits_lod_when_main_oversized(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/a.bto", 4000, tmp_path),
            _entry("Scripts/a.pex", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - LOD.ba2",
        "B21_Test - Scripts.ba2",
    ]
    assert [plan.family for plan in plans] == ["LOD", "Scripts"]


def test_plan_archive_outputs_shards_oversized_lod(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/a.bto", 4000, tmp_path),
            _entry("Meshes/Terrain/Appalachia/b.bto", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - LOD1.ba2",
        "B21_Test - LOD2.ba2",
    ]
    assert [plan.family for plan in plans] == ["LOD", "LOD"]


def test_plan_archive_outputs_shards_oversized_scripts(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Scripts/a.pex", 4000, tmp_path),
            _entry("Scripts/b.pex", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Scripts1.ba2",
        "B21_Test - Scripts2.ba2",
    ]
    assert [plan.family for plan in plans] == ["Scripts", "Scripts"]


def test_plan_archive_outputs_keeps_split_strings_in_main_archive(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Strings/B21_Test_en.STRINGS", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Main.ba2",
    ]
    assert plans[1].family == "Main"
    assert [entry.relative_path for entry in plans[1].entries] == [
        "Strings/B21_Test_en.STRINGS",
    ]


def test_plan_archive_outputs_shards_textures(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Textures/a.dds", 4000, tmp_path),
            _entry("Textures/b.dds", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Textures1.ba2",
        "B21_Test - Textures2.ba2",
    ]
    assert all(plan.texture_archive for plan in plans)


def test_plan_archive_outputs_shards_oversized_category(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Meshes/b.nif", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes1.ba2",
        "B21_Test - Meshes2.ba2",
    ]


def test_plan_archive_outputs_puts_xbox_suffix_after_label(tmp_path: Path):
    plans = plan_archive_outputs(
        "B21_Test",
        [_entry("Textures/a.dds", 10, tmp_path)],
        ".ba2",
        "_xbox",
        1024 * 1024,
    )

    assert [plan.output_name for plan in plans] == ["B21_Test - Textures_xbox.ba2"]


def test_plan_archive_outputs_rejects_single_file_over_cap(tmp_path: Path):
    entry = _entry("Meshes/a.nif", 7000, tmp_path)
    with pytest.raises(ValueError, match="exceeding archive max size"):
        plan_archive_outputs("B21_Test", [entry], "ba2", "", 9000)


def test_discover_mod_archives_matches_mod_prefix_and_extensions(tmp_path: Path):
    expected = [
        tmp_path / "B21_Test - Main.ba2",
        tmp_path / "B21_Test - Meshes1.ba2",
        tmp_path / "B21_Test - MeshesExtra.ba2",
        tmp_path / "B21_Test - MeshesExtra2.ba2",
        tmp_path / "B21_Test - Misc.ba2",
        tmp_path / "B21_Test - Misc2.ba2",
        tmp_path / "B21_Test - Textures.bsa",
    ]
    for path in expected:
        path.write_bytes(b"archive")
    (tmp_path / "B21_Test.ba2").write_bytes(b"no label")
    (tmp_path / "B21_Test - HiRes.ba2").write_bytes(b"manual")
    (tmp_path / "Other - Main.ba2").write_bytes(b"other")
    (tmp_path / "B21_Test - Main.zip").write_bytes(b"zip")

    assert discover_mod_archives(tmp_path, "B21_Test") == expected


def test_discover_mod_archives_includes_generated_lod_archives(tmp_path: Path):
    expected = [
        tmp_path / "B21_Test - LOD.ba2",
        tmp_path / "B21_Test - LOD1.ba2",
        tmp_path / "B21_Test - LODTextures.ba2",
        tmp_path / "B21_Test - LODTextures2.ba2",
        tmp_path / "B21_Test - Main.ba2",
        tmp_path / "B21_Test - TerrainTextures.ba2",
    ]
    for path in expected:
        path.write_bytes(b"archive")
    (tmp_path / "B21_Test - HiRes.ba2").write_bytes(b"manual")

    assert discover_mod_archives(tmp_path, "B21_Test") == expected
