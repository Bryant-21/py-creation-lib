from creation_lib.nif.operations.collision_materials import (
    build_maxscript_collision_material_defs,
    collision_material_type_name,
    default_collision_material,
    format_collision_material,
    get_collision_material_options,
    resolve_collision_material,
)


def test_collision_material_resolves_fo4_names_and_labels():
    assert resolve_collision_material("MaterialWeaponPistol") == 4146539321
    assert resolve_collision_material("WeaponPistol") == 4146539321
    assert resolve_collision_material("Generic") == 186875565
    assert default_collision_material() == 186875565


def test_collision_material_options_include_weapon_pistol():
    options = get_collision_material_options()
    assert {"name": "MaterialWeaponPistol", "label": "WeaponPistol", "value": 4146539321} in options
    assert format_collision_material(4146539321) == "WeaponPistol (4146539321)"
    assert collision_material_type_name(4146539321) == "MaterialWeaponPistol"


def test_collision_material_options_pin_null_and_generic_then_sort_by_label():
    options = get_collision_material_options()
    assert [option["label"] for option in options[:2]] == ["NullMaterial", "Generic"]
    assert int(options[1]["value"]) == default_collision_material()

    remaining = options[2:]
    assert remaining == sorted(
        remaining,
        key=lambda option: (str(option["label"]).casefold(), str(option["name"]).casefold()),
    )


def test_collision_material_maxscript_defs_use_shared_order():
    script = build_maxscript_collision_material_defs()
    assert "global MB21_NIF_COLLISION_MATERIAL_LABELS" in script
    assert 'MB21_NIF_COLLISION_MATERIAL_LABELS = #("NullMaterial", "Generic", "ActorArmored"' in script
    assert '"WeaponPistol"' in script
