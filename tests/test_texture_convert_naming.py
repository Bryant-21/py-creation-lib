import pytest


def test_rename_fo76_to_fo4():
    """FO76 _r (reflectivity) -> FO4 _s (specular) via fallback.
    FO76 _l (lighting/emissive rolloff) -> FO4 _g (glow) via fallback, since
    the BGSM downgrade promotes LightingTexture into GlowTexture.
    """
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core.game_profiles import FO76_PROFILE, FO4_PROFILE

    assert convert_texture_name("armor_l.dds", FO76_PROFILE, FO4_PROFILE) == "armor_g.dds"
    assert convert_texture_name("armor_r.dds", FO76_PROFILE, FO4_PROFILE) == "armor_s.dds"


def test_rename_fo4_to_fo76():
    """FO4 _s suffix should map to FO76 _r (metallic)."""
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core.game_profiles import FO76_PROFILE, FO4_PROFILE

    # _s maps to _r (the primary metallic map)
    assert convert_texture_name("armor_s.dds", FO4_PROFILE, FO76_PROFILE) == "armor_r.dds"


def test_rename_shared_suffixes_unchanged():
    """Suffixes shared between games should pass through."""
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core.game_profiles import FO76_PROFILE, FO4_PROFILE

    # _d (diffuse) and _n (normal) are shared
    assert convert_texture_name("armor_d.dds", FO4_PROFILE, FO76_PROFILE) == "armor_d.dds"
    assert convert_texture_name("armor_n.dds", FO4_PROFILE, FO76_PROFILE) == "armor_n.dds"


def test_rename_starfield_to_fo4():
    """Starfield _color/_rough/_normal suffixes -> FO4 _d/_s/_n."""
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core.game_profiles import STARFIELD_PROFILE, FO4_PROFILE

    assert convert_texture_name("gun_color.dds", STARFIELD_PROFILE, FO4_PROFILE) == "gun_d.dds"
    assert convert_texture_name("gun_normal.dds", STARFIELD_PROFILE, FO4_PROFILE) == "gun_n.dds"


def test_rename_no_recognized_suffix():
    """Filenames without recognized suffixes pass through unchanged."""
    from creation_lib.textures.naming import convert_texture_name
    from creation_lib.core.game_profiles import FO4_PROFILE, FO76_PROFILE

    assert convert_texture_name("custom_texture.dds", FO4_PROFILE, FO76_PROFILE) == "custom_texture.dds"


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
