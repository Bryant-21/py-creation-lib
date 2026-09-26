import pytest


@pytest.mark.parametrize("filename,src_profile,dst_profile,expected", [
    # FO76 _r (reflectivity) -> FO4 _s (specular) via fallback.
    ("armor_r.dds", "FO76_PROFILE", "FO4_PROFILE", "armor_s.dds"),
    # FO76 _l (lighting/emissive rolloff) -> FO4 _g (glow) via fallback, since
    # the BGSM downgrade promotes LightingTexture into GlowTexture.
    ("armor_l.dds", "FO76_PROFILE", "FO4_PROFILE", "armor_g.dds"),
    # FO4 _s maps to FO76 _r (the primary metallic map).
    ("armor_s.dds", "FO4_PROFILE", "FO76_PROFILE", "armor_r.dds"),
    # Suffixes shared between games (_d diffuse, _n normal) pass through.
    ("armor_d.dds", "FO4_PROFILE", "FO76_PROFILE", "armor_d.dds"),
    ("armor_n.dds", "FO4_PROFILE", "FO76_PROFILE", "armor_n.dds"),
    # Starfield _color/_normal -> FO4 _d/_n.
    ("gun_color.dds", "STARFIELD_PROFILE", "FO4_PROFILE", "gun_d.dds"),
    ("gun_normal.dds", "STARFIELD_PROFILE", "FO4_PROFILE", "gun_n.dds"),
    # Unrecognized suffixes pass through unchanged.
    ("custom_texture.dds", "FO4_PROFILE", "FO76_PROFILE", "custom_texture.dds"),
])
def test_convert_texture_name_cases(filename, src_profile, dst_profile, expected):
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core import game_profiles

    src = getattr(game_profiles, src_profile)
    dst = getattr(game_profiles, dst_profile)
    assert convert_texture_name(filename, src, dst) == expected


def test_detect_texture_role():
    """Detect the semantic role of a texture from its filename suffix."""
    from creation_lib.textures.naming import detect_texture_role
    from creation_lib.core.game_profiles import FO4_PROFILE, FO76_PROFILE, STARFIELD_PROFILE

    assert detect_texture_role("armor_d.dds", FO4_PROFILE) == "diffuse"
    assert detect_texture_role("armor_n.dds", FO4_PROFILE) == "normal"
    assert detect_texture_role("armor_s.dds", FO4_PROFILE) == "specular"
    assert detect_texture_role("armor_l.dds", FO76_PROFILE) == "lighting"
    assert detect_texture_role("armor_r.dds", FO76_PROFILE) == "reflectivity"
    assert detect_texture_role("gun_color.dds", STARFIELD_PROFILE) == "diffuse"
    assert detect_texture_role("gun_metal.dds", STARFIELD_PROFILE) == "metallic"
    assert detect_texture_role("unknown.dds", FO4_PROFILE) is None
