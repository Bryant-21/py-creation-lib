"""Tests for creation_lib.havok.discovery — file walking and role classification."""
import os
import tempfile
from pathlib import Path

import pytest

from creation_lib.havok.discovery import (
    FileEntry,
    classify_category,
    classify_role,
    discover_havok_files,
)


def _make_tree(base: Path, paths: list[str]):
    """Create empty files at given relative paths."""
    for p in paths:
        fp = base / p.replace("/", os.sep)
        fp.parent.mkdir(parents=True, exist_ok=True)
        fp.write_text("<xml/>")


class TestClassifyRole:
    def test_project_sibling_to_behaviors(self, tmp_path):
        _make_tree(tmp_path, [
            "UniqueBehaviors/FlamerFX/FlamerFX.xml",
            "UniqueBehaviors/FlamerFX/Behaviors/Behavior.xml",
            "UniqueBehaviors/FlamerFX/Characters/Character.xml",
        ])
        assert classify_role(
            "UniqueBehaviors/FlamerFX/FlamerFX.xml", tmp_path
        ) == "project"

    def test_character_in_characters_dir(self):
        assert classify_role(
            "UniqueBehaviors/FlamerFX/Characters/Character.xml"
        ) == "character"

    def test_behavior_in_behaviors_dir(self):
        assert classify_role(
            "UniqueBehaviors/FlamerFX/Behaviors/Behavior.xml"
        ) == "behavior"

    def test_skeleton_hkx_in_character_assets(self):
        assert classify_role(
            "Actors/Deathclaw/CharacterAssets/skeleton.xml"
        ) == "skeleton"

    def test_skeleton_hkt(self):
        assert classify_role(
            "GenericBehaviors/zSingleBoneSkeleton/SingleBoneSkeleton.hkt"
        ) == "skeleton"

    def test_animation_in_animations_dir(self):
        assert classify_role(
            "Actors/Character/Animations/1HM/AttackA.xml"
        ) == "animation"

    def test_nif_is_asset(self):
        assert classify_role(
            "Actors/Deathclaw/CharacterAssets/skeleton.nif"
        ) == "asset"

    def test_dds_is_asset(self):
        assert classify_role(
            "Actors/Deathclaw/CharacterAssets/body.dds"
        ) == "asset"


class TestClassifyCategory:
    def test_unique_behaviors(self):
        assert classify_category("UniqueBehaviors/ShishkebabFX/Behavior.xml") == "Weapon"

    def test_generic_behaviors(self):
        assert classify_category("GenericBehaviors/Workshop/Behavior.xml") == "Generic"

    def test_actor_character(self):
        assert classify_category("Actors/Character/Behaviors/WeaponBehavior.xml") == "Character"

    def test_actor_creature(self):
        assert classify_category("Actors/Deathclaw/Behaviors/Behavior.xml") == "Creature"

    def test_dlc_behaviors_unique(self):
        assert classify_category("DLC01/BehaviorsUnique/LightningRifle/Behavior.xml") == "Weapon"

    def test_fallback_misc(self):
        assert classify_category("SomeRandomPath/Behavior.xml") == "Misc"


class TestDiscoverHavokFiles:
    def test_discovers_all_roles(self, tmp_path):
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
        assert "project" in roles
        assert "character" in roles
        assert "behavior" in roles
        assert "skeleton" in roles
        assert "animation" in roles
        assert "asset" in roles

    def test_entry_has_required_fields(self, tmp_path):
        _make_tree(tmp_path, [
            "UniqueBehaviors/TestFX/Behaviors/Behavior.xml",
        ])
        entries = list(discover_havok_files(tmp_path))
        assert len(entries) == 1
        e = entries[0]
        assert e.rel_path == "UniqueBehaviors/TestFX/Behaviors/Behavior.xml"
        assert e.role == "behavior"
        assert e.category == "Weapon"
        assert e.abs_path == tmp_path / "UniqueBehaviors" / "TestFX" / "Behaviors" / "Behavior.xml"
