from creation_lib.nif.nif_bsx_flags import (
    BSX_FLAG_DEFS,
    bsx_flags_to_bits,
    build_maxscript_bsx_flag_defs,
)


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
