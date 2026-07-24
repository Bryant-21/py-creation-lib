from pathlib import Path

from creation_lib.preprocessor import extraction
from creation_lib.preprocessor.extraction import find_archives, group_archives_by_update_phase


def _touch_archives(data_dir: Path, names: list[str]) -> None:
    data_dir.mkdir(parents=True)
    for name in names:
        (data_dir / name).write_bytes(b"archive")


def test_numbered_update_archives_sort_after_base_archives(tmp_path: Path) -> None:
    data_dir = tmp_path / "Data"
    _touch_archives(
        data_dir,
        [
            "SeventySix - 13UpdateMain.ba2",
            "SeventySix - 03UpdateVoices.ba2",
            "SeventySix - Textures10.ba2",
            "SeventySix - Textures01.ba2",
            "SeventySix - 02UpdateTextures.ba2",
            "SeventySix - 01UpdateMain.ba2",
            "SeventySix - 00UpdateTextures.ba2",
            "SeventySix - 14UpdateMaterials.ba2",
            "SeventySix - Materials.ba2",
        ],
    )

    assert [archive.name for archive in find_archives(data_dir, "ba2")] == [
        "SeventySix - Materials.ba2",
        "SeventySix - Textures01.ba2",
        "SeventySix - Textures10.ba2",
        "SeventySix - 00UpdateTextures.ba2",
        "SeventySix - 01UpdateMain.ba2",
        "SeventySix - 02UpdateTextures.ba2",
        "SeventySix - 03UpdateVoices.ba2",
        "SeventySix - 13UpdateMain.ba2",
        "SeventySix - 14UpdateMaterials.ba2",
    ]


def test_update_archives_form_ordered_parallel_phases(tmp_path: Path) -> None:
    data_dir = tmp_path / "Data"
    _touch_archives(
        data_dir,
        [
            "SeventySix - 02UpdateTextures.ba2",
            "SeventySix - Textures02.ba2",
            "SeventySix - 01UpdateMain.ba2",
            "SeventySix - 00UpdateMain.ba2",
            "SeventySix - Textures01.ba2",
            "SeventySix - 13UpdateVoices.ba2",
        ],
    )

    groups = group_archives_by_update_phase(find_archives(data_dir, "ba2"))

    assert [[archive.name for archive in group] for group in groups] == [
        ["SeventySix - Textures01.ba2", "SeventySix - Textures02.ba2"],
        ["SeventySix - 00UpdateMain.ba2"],
        ["SeventySix - 01UpdateMain.ba2"],
        ["SeventySix - 02UpdateTextures.ba2"],
        ["SeventySix - 13UpdateVoices.ba2"],
    ]


def test_archive_batches_respect_total_worker_budget(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "Data"
    _touch_archives(
        data_dir,
        [
            "SmallA.ba2",
            "HugeMeshes.ba2",
            "SmallB.ba2",
            "LargeTextures.ba2",
        ],
    )
    counts = {
        "SmallA.ba2": 100,
        "HugeMeshes.ba2": 60_539,
        "SmallB.ba2": 100,
        "LargeTextures.ba2": 22_000,
    }
    monkeypatch.setattr(
        extraction,
        "archive_entry_count",
        lambda archive: counts[archive.name],
    )

    batches = extraction.plan_archive_extraction_batches(find_archives(data_dir, "ba2"), 8)

    assert [[task.archive.name for task in batch] for batch in batches] == [
        ["HugeMeshes.ba2"],
        ["LargeTextures.ba2", "SmallA.ba2", "SmallB.ba2"],
    ]
    assert [[task.file_workers for task in batch] for batch in batches] == [
        [7],
        [3, 1, 1],
    ]
    assert all(sum(task.file_workers for task in batch) <= 8 for batch in batches)


def test_large_archives_receive_the_full_worker_budget(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "Data"
    _touch_archives(
        data_dir,
        [
            "Textures01.ba2",
            "Textures02.ba2",
        ],
    )
    monkeypatch.setattr(extraction, "archive_entry_count", lambda _archive: 8_000)
    monkeypatch.setattr(
        extraction,
        "archive_size_bytes",
        lambda _archive: 4 * 1024**3,
    )

    batches = extraction.plan_archive_extraction_batches(
        find_archives(data_dir, "ba2"),
        8,
    )

    assert [[task.archive.name for task in batch] for batch in batches] == [
        ["Textures01.ba2"],
        ["Textures02.ba2"],
    ]
    assert [[task.file_workers for task in batch] for batch in batches] == [[8], [8]]
