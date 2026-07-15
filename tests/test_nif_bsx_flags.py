from creation_lib.nif.nif_bsx_flags import (
    BSX_FLAG_DEFS,
    bsx_flags_to_bits,
    build_maxscript_bsx_flag_defs,
)


def test_bsx_flag_defs_match_expected_known_bits():
    assert [flag.bit for flag in BSX_FLAG_DEFS] == list(range(14))
    assert BSX_FLAG_DEFS[0].label == "Animated"
    assert BSX_FLAG_DEFS[8].label == "Needs Transform Updates"
    assert BSX_FLAG_DEFS[13].label == "Searched Breakable"


def test_bsx_flags_to_bits_returns_set_bits_in_definition_order():
    assert bsx_flags_to_bits((1 << 0) | (1 << 5) | (1 << 9)) == [0, 5, 9]


def test_build_maxscript_bsx_flag_defs_emits_shared_array():
    script = build_maxscript_bsx_flag_defs()

    assert "global MB21_NIF_BSX_FLAG_DEFS" in script
    assert '#(0, "Animated", 1, "Enable havok / bAnimated")' in script
    assert (
        '#(13, "Searched Breakable", 8192, "bSearchedBreakable (runtime-only in some games)")'
        in script
    )
