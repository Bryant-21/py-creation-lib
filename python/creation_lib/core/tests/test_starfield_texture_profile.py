from creation_lib.core.game_profiles import get_profile


def test_starfield_profile_has_texture_remix():
    profile = get_profile("starfield")
    assert profile.texture_slot_map, "starfield texture_slot_map must not be empty"
    assert profile.texture_remix is not None
    fo76 = get_profile("fo76")
    assert type(profile.texture_remix) is type(fo76.texture_remix)


def test_starfield_slot_map_covers_pbr_suffixes():
    slot_map = get_profile("starfield").texture_slot_map
    for suffix in ("_color", "_normal", "_rough", "_metal", "_ao", "_emissive"):
        assert suffix in slot_map, suffix
