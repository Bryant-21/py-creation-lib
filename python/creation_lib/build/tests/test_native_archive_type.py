import pytest

from creation_lib.build.packer import _native_archive_type


@pytest.mark.parametrize(
    ("game", "textures", "kwargs", "expected"),
    [
        ("fo4", False, {}, "fo4"),
        ("fo4", True, {}, "fo4dds"),
        ("fo4", False, {"og": True}, "fo4og"),
        ("fo4", True, {"og": True}, "fo4ogdds"),
        ("fo76", False, {"og": True}, "fo76"),
        ("fo4", True, {"xbox": True, "og": True}, "fo4xboxdds"),
        ("fo4", False, {"ps": True}, "fo4ps"),
        ("fo4", True, {"ps": True}, "fo4psdds"),
    ],
)
def test_native_archive_type_tokens(game, textures, kwargs, expected):
    assert _native_archive_type(game, textures, **kwargs) == expected
