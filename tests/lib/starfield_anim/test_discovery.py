import pytest
from pathlib import Path
from creation_lib.havok.discovery import classify_role, classify_category, HAVOK_EXTENSIONS

def test_rig_extension_classified_as_skeleton():
    assert classify_role("actors/human/characterassets/skeleton.rig") == "skeleton"

def test_af_extension_classified_as_animation():
    assert classify_role("actors/human/animations/idle.af") == "animation"

def test_agx_extension_classified_as_behavior():
    assert classify_role("animtextdata/tables/graphs/simpleidleloop01.agx") == "behavior"

def test_starfield_extensions_in_havok_extensions():
    assert ".af" in HAVOK_EXTENSIONS
    assert ".rig" in HAVOK_EXTENSIONS
    assert ".agx" in HAVOK_EXTENSIONS

def test_animtextdata_category():
    assert classify_category("animtextdata/tables/graphs/foo.agx") == "AnimGraph"

def test_rig_name_not_skeleton_still_classified():
    """A .rig file not named 'skeleton' should still be role=skeleton."""
    assert classify_role("actors/human/characterassets/humanmale.rig") == "skeleton"
