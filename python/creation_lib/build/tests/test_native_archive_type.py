from creation_lib.build.packer import _native_archive_type


def test_fo4_nextgen_tokens():
    assert _native_archive_type("fo4", False) == "fo4"
    assert _native_archive_type("fo4", True) == "fo4dds"


def test_fo4_og_tokens():
    assert _native_archive_type("fo4", False, og=True) == "fo4og"
    assert _native_archive_type("fo4", True, og=True) == "fo4ogdds"


def test_og_flag_ignored_for_non_fo4():
    assert _native_archive_type("fo76", False, og=True) == "fo76"


def test_xbox_precedence_over_og():
    assert _native_archive_type("fo4", True, xbox=True, og=True) == "fo4xboxdds"
