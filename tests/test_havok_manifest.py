"""Tests for creation_lib.havok.manifest — asset manifest assembly."""
import os
from pathlib import Path

import pytest

from creation_lib.havok.discovery import FileEntry
from creation_lib.havok.manifest import (
    ManifestData,
    ManifestFileEntry,
    ManifestDep,
    build_manifests,
)


def _entry(rel_path: str, role: str, category: str = "Weapon",
           abs_path: Path = Path("/fake")) -> FileEntry:
    """Helper to create FileEntry for tests."""
    ext = os.path.splitext(rel_path)[1].lstrip(".")
    return FileEntry(
        abs_path=abs_path / rel_path,
        rel_path=rel_path,
        role=role,
        category=category,
        file_type=ext,
        is_xml=ext == "xml",
    )


class TestBuildManifests:
    def test_unique_behavior_manifest(self):
        entries = [
            _entry("UniqueBehaviors/FlamerFX/FlamerFX.xml", "project"),
            _entry("UniqueBehaviors/FlamerFX/Characters/Character.xml", "character"),
            _entry("UniqueBehaviors/FlamerFX/Behaviors/Behavior.xml", "behavior"),
        ]
        manifests = build_manifests(entries, {}, "fo4")
        assert len(manifests) >= 1
        m = next(m for m in manifests if "FlamerFX" in m.id)
        assert m.manifest_type == "weapon_fx"
        assert len(m.files) == 3

    def test_actor_manifest(self):
        entries = [
            _entry("Actors/Deathclaw/DeathclawProject.xml", "project", "Creature"),
            _entry("Actors/Deathclaw/Characters/Character.xml", "character", "Creature"),
            _entry("Actors/Deathclaw/CharacterAssets/skeleton.xml", "skeleton", "Creature"),
            _entry("Actors/Deathclaw/Behaviors/Behavior.xml", "behavior", "Creature"),
            _entry("Actors/Deathclaw/Animations/Attack.xml", "animation", "Creature"),
            _entry("Actors/Deathclaw/CharacterAssets/body.nif", "asset", "Creature"),
        ]
        manifests = build_manifests(entries, {}, "fo4")
        m = next(m for m in manifests if "Deathclaw" in m.id)
        assert m.manifest_type == "actor"
        assert len(m.files) == 6

    def test_shared_skeleton_dependency(self):
        """Weapon FX referencing zSingleBoneSkeleton should create a dependency."""
        from creation_lib.havok.parsers.character import CharacterData
        char_data = {
            "UniqueBehaviors/FlamerFX/Characters/Character.xml": CharacterData(
                rig_name="..\\..\\GenericBehaviors\\zSingleBoneSkeleton\\SingleBoneSkeleton.hkt"
            ),
        }
        entries = [
            _entry("UniqueBehaviors/FlamerFX/FlamerFX.xml", "project"),
            _entry("UniqueBehaviors/FlamerFX/Characters/Character.xml", "character"),
            _entry("UniqueBehaviors/FlamerFX/Behaviors/Behavior.xml", "behavior"),
        ]
        manifests = build_manifests(entries, char_data, "fo4")
        m = next(m for m in manifests if "FlamerFX" in m.id)
        skel_deps = [d for d in m.dependencies if d.dep_type == "skeleton"]
        assert len(skel_deps) >= 1

    def test_file_ref_types(self):
        entries = [
            _entry("UniqueBehaviors/TestFX/TestFX.xml", "project"),
            _entry("UniqueBehaviors/TestFX/Behaviors/Behavior.xml", "behavior"),
        ]
        manifests = build_manifests(entries, {}, "fo4")
        m = next(m for m in manifests if "TestFX" in m.id)
        assert all(f.ref_type == "owned" for f in m.files)
