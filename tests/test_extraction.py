from pathlib import Path

from creation_lib.preprocessor.extraction import build_manifest, manifest_matches, sync_papyrus_sources


def test_sync_papyrus_sources_mirrors_into_scripts_source(tmp_path: Path):
    source_dir = tmp_path / "game" / "Data" / "Scripts" / "Source"
    source_dir.mkdir(parents=True)
    (source_dir / "BaseScript.psc").write_text("Scriptname BaseScript\n", encoding="utf-8")
    (source_dir / "Subdir").mkdir()
    (source_dir / "Subdir" / "NestedScript.psc").write_text("Scriptname NestedScript\n", encoding="utf-8")

    output_dir = tmp_path / "extracted" / "fo4"
    count = sync_papyrus_sources(source_dir, output_dir)

    assert count == 2
    assert (output_dir / "Scripts" / "Source" / "BaseScript.psc").is_file()
    assert (output_dir / "Scripts" / "Source" / "Subdir" / "NestedScript.psc").is_file()


def test_manifest_matches_detects_papyrus_source_changes(tmp_path: Path):
    data_dir = tmp_path / "game" / "Data"
    data_dir.mkdir(parents=True)
    archive = data_dir / "Fallout4 - Main.ba2"
    archive.write_bytes(b"ba2")

    papyrus_dir = data_dir / "Scripts" / "Source"
    papyrus_dir.mkdir(parents=True)
    script = papyrus_dir / "TestScript.psc"
    script.write_text("Scriptname TestScript\n", encoding="utf-8")

    manifest = build_manifest("fo4", data_dir, [archive], papyrus_dir)
    assert manifest_matches(manifest, data_dir, [archive], papyrus_dir)

    script.write_text("Scriptname TestScript\nFunction Foo()\nEndFunction\n", encoding="utf-8")
    assert not manifest_matches(manifest, data_dir, [archive], papyrus_dir)
