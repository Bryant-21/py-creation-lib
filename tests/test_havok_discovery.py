"""Smoke test for creation_lib.havok.discovery + manifest — file walking, role
classification and manifest assembly. Detailed classify/manifest cases are
covered by the Rust havok_assets.rs tests; this keeps one synthetic Python
smoke test to confirm the Python bindings still exercise the same logic."""
import os
from pathlib import Path

from creation_lib.havok.discovery import discover_havok_files
from creation_lib.havok.manifest import build_manifests


def _make_tree(base: Path, paths: list[str]):
    for p in paths:
        fp = base / p.replace("/", os.sep)
        fp.parent.mkdir(parents=True, exist_ok=True)
        fp.write_text("<xml/>")


def test_discover_and_build_manifest(tmp_path):
    _make_tree(tmp_path, [
        "UniqueBehaviors/TestFX/TestFX.xml",
        "UniqueBehaviors/TestFX/Characters/Character.xml",
        "UniqueBehaviors/TestFX/Behaviors/Behavior.xml",
        "Actors/TestCreature/CharacterAssets/skeleton.xml",
        "Actors/TestCreature/Animations/Attack.xml",
        "Actors/TestCreature/CharacterAssets/body.nif",
    ])
    entries = list(discover_havok_files(tmp_path))
    roles = {e.role for e in entries}
    assert roles == {"project", "character", "behavior", "skeleton", "animation", "asset"}

    manifests = build_manifests(entries, {}, "fo4")
    m = next(m for m in manifests if "TestFX" in m.id)
    assert m.manifest_type == "weapon_fx"
    assert len(m.files) == 3
