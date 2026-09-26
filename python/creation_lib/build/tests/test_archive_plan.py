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


def test_gib_to_bytes_converts_and_rejects_non_positive():
    assert DEFAULT_ARCHIVE_MAX_BYTES == 16 * 1024**3
    assert gib_to_bytes(16.0) == DEFAULT_ARCHIVE_MAX_BYTES
    with pytest.raises(ValueError):
        gib_to_bytes(0)
    with pytest.raises(ValueError):
        gib_to_bytes(-1)


@pytest.mark.parametrize(
    ("path", "family"),
    [
        ("Textures/foo.dds", "Textures"),
        ("Interface/menu.swf", "Interface"),
        ("Materials/foo.bgsm", "Materials"),
        ("Meshes/foo/material.bgem", "Materials"),
        ("Strings/B21_en.STRINGS", "Strings"),
        ("data/foo.dlstrings", "Strings"),
        ("Sound/fx/foo.xwm", "Sounds"),
        ("Meshes/Actors/Anim.hkx", "Animations"),
        ("Meshes/foo.nif", "Meshes"),
        ("Scripts/foo.pex", "Scripts"),
        ("Meshes/Terrain/Appalachia/Foo.bto", "LOD"),
        ("data/Meshes/Terrain/Appalachia/Foo.BTO", "LOD"),
    ],
)
def test_classify_archive_family(path: str, family: str):
    assert classify_archive_family(path) == family


def test_plan_archive_outputs_preserves_small_names_keeps_lod_in_main_and_sorts_entries(
    tmp_path: Path,
):
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

    # A small LOD family stays folded into Main rather than getting its own archive.
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

    # Entries land sorted by relative path regardless of input order.
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
    assert [entry.relative_path for entry in plans[1].entries] == [
        "Scripts/test.pex",
        "readme.txt",
    ]


def test_plan_archive_outputs_routes_land_assets_to_generic_archives_when_expanded_and_compacts_them_otherwise(
    tmp_path: Path,
):
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
    assert [entry.relative_path for entry in by_label["Materials"].entries] == [
        "Materials/Terrain/Appalachia/blend.bgsm"
    ]
    assert [entry.relative_path for entry in by_label["Textures"].entries] == [
        "Textures/Terrain/Appalachia/lswamprocks01_d.dds"
    ]

    # With expanded_archives off, LOD and terrain dds compact into Main/Textures.
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/Objects/a.bto", 10, tmp_path),
            _entry("Textures/Actors/a.dds", 10, tmp_path),
            _entry("Textures/Terrain/Appalachia/Appalachia.4.0.0.dds", 10, tmp_path),
            _entry("Materials/Terrain/Appalachia/blend.bgsm", 10, tmp_path),
            _entry("Textures/Terrain/Appalachia/lswamprocks01_d.dds", 10, tmp_path),
        ],
        "ba2",
        "",
        1024 * 1024,
        game="fo4",
        expanded_archives=False,
    )

    assert [plan.label for plan in plans] == ["Main", "Textures"]
    assert [entry.relative_path for entry in plans[1].entries] == [
        "Textures/Actors/a.dds",
        "Textures/Terrain/Appalachia/Appalachia.4.0.0.dds",
        "Textures/Terrain/Appalachia/lswamprocks01_d.dds",
    ]
    assert plans[1].texture_archive is True


@pytest.mark.parametrize(
    ("entries", "max_bytes", "game", "expected_names", "expected_labels", "expected_error"),
    [
        (
            [("Sound/a.fuz", 5000), ("Sound/b.fuz", 5000)],
            14_000,
            None,
            ["B21_Test - Sounds1.ba2", "B21_Test - Sounds2.ba2"],
            None,
            None,
        ),
        (
            [("Scripts/a.pex", 4000), ("Scripts/b.pex", 4000)],
            9000,
            None,
            ["B21_Test - Scripts1.ba2", "B21_Test - Scripts2.ba2"],
            None,
            None,
        ),
        (
            [("Textures/a.dds", 4000), ("Textures/b.dds", 4000)],
            9000,
            None,
            ["B21_Test - Textures1.ba2", "B21_Test - Textures2.ba2"],
            None,
            None,
        ),
        (
            [("Meshes/a.nif", 4000), ("Meshes/b.nif", 4000)],
            9000,
            None,
            ["B21_Test - Meshes1.ba2", "B21_Test - Meshes2.ba2"],
            None,
            None,
        ),
        (
            [("Meshes/a.nif", 4000), ("Meshes/b.nif", 4000)],
            9000,
            "fo4",
            ["B21_Test - Meshes.ba2", "B21_Test - MeshesExtra.ba2"],
            ["Meshes", "MeshesExtra"],
            None,
        ),
        (
            [("Scripts/a.pex", 4000), ("Scripts/b.pex", 4000), ("Scripts/c.pex", 4000)],
            9000,
            "fo4",
            [
                "B21_Test - Misc.ba2",
                "B21_Test - Misc1.ba2",
                "B21_Test - Misc2.ba2",
            ],
            ["Misc", "Misc1", "Misc2"],
            None,
        ),
        (
            [("Meshes/a.nif", 7000)],
            9000,
            None,
            None,
            None,
            "exceeding archive max size",
        ),
    ],
)
def test_plan_archive_outputs_shards_and_numbers_oversized_families(
    tmp_path: Path, entries, max_bytes, game, expected_names, expected_labels, expected_error
):
    kwargs = {"expanded_archives": True}
    if game is not None:
        kwargs["game"] = game

    def call():
        return plan_archive_outputs(
            "B21_Test",
            [_entry(path, size, tmp_path) for path, size in entries],
            "ba2",
            "",
            max_bytes,
            **kwargs,
        )

    if expected_error is not None:
        with pytest.raises(ValueError, match=expected_error):
            call()
        return

    plans = call()
    assert [plan.output_name for plan in plans] == expected_names
    if expected_labels is not None:
        assert [plan.label for plan in plans] == expected_labels


def test_plan_archive_outputs_splits_lod_when_main_oversized_and_keeps_split_strings_in_main(
    tmp_path: Path,
):
    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/Terrain/Appalachia/a.bto", 4000, tmp_path),
            _entry("Scripts/a.pex", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - LOD.ba2",
        "B21_Test - Scripts.ba2",
    ]
    assert [plan.family for plan in plans] == ["LOD", "Scripts"]

    plans = plan_archive_outputs(
        "B21_Test",
        [
            _entry("Meshes/a.nif", 4000, tmp_path),
            _entry("Strings/B21_Test_en.STRINGS", 4000, tmp_path),
        ],
        "ba2",
        "",
        9000,
        expanded_archives=True,
    )

    assert [plan.output_name for plan in plans] == [
        "B21_Test - Meshes.ba2",
        "B21_Test - Main.ba2",
    ]
    assert plans[1].family == "Main"


@pytest.mark.parametrize(
    ("suffix", "expected_name"),
    [
        ("_xbox", "B21_Test - Textures_xbox.ba2"),
        ("_ps", "B21_Test - Textures_ps.ba2"),
    ],
)
def test_plan_archive_outputs_puts_platform_suffix_after_label(
    tmp_path: Path, suffix, expected_name
):
    plans = plan_archive_outputs(
        "B21_Test",
        [_entry("Textures/a.dds", 10, tmp_path)],
        ".ba2",
        suffix,
        1024 * 1024,
    )

    assert [plan.output_name for plan in plans] == [expected_name]


def test_discover_mod_archives_matches_mod_prefix_extensions_and_generated_lod(tmp_path: Path):
    expected = [
        tmp_path / "B21_Test - LOD.ba2",
        tmp_path / "B21_Test - Main.ba2",
        tmp_path / "B21_Test - Meshes1.ba2",
        tmp_path / "B21_Test - MeshesExtra.ba2",
        tmp_path / "B21_Test - MeshesExtra2.ba2",
        tmp_path / "B21_Test - Misc.ba2",
        tmp_path / "B21_Test - Misc2.ba2",
        tmp_path / "B21_Test - Textures.bsa",
        tmp_path / "B21_Test - Textures_ps.ba2",
    ]
    for path in expected:
        path.write_bytes(b"archive")
    (tmp_path / "B21_Test.ba2").write_bytes(b"no label")
    (tmp_path / "B21_Test - HiRes.ba2").write_bytes(b"manual")
    (tmp_path / "Other - Main.ba2").write_bytes(b"other")
    (tmp_path / "B21_Test - Main.zip").write_bytes(b"zip")

    assert discover_mod_archives(tmp_path, "B21_Test") == expected
