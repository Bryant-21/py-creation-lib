"""Tests for Havok version registry."""
import pytest
from creation_lib.havok_convert.versions import (
    HavokVersion, get_version, get_version_by_name,
    get_version_chain, SKYRIM_SE, FO4, FO76,
)


def test_known_versions():
    assert FO4.id == 53
    assert FO4.name == "hk_2014.1.0-r1"
    assert SKYRIM_SE.id == 46
    assert FO76.id == 56


def test_get_version_by_id():
    v = get_version(53)
    assert v.name == "hk_2014.1.0-r1"


def test_get_version_by_name():
    v = get_version_by_name("hk_2014.1.0-r1")
    assert v.id == 53


def test_version_chain_upgrade():
    chain = get_version_chain(46, 56)
    assert len(chain) > 0
    assert chain[0] == 46
    assert chain[-1] == 56
    # Each step increments
    for i in range(1, len(chain)):
        assert chain[i] > chain[i - 1]


def test_version_chain_downgrade():
    chain = get_version_chain(56, 46)
    assert chain[0] == 56
    assert chain[-1] == 46
    for i in range(1, len(chain)):
        assert chain[i] < chain[i - 1]


def test_version_chain_same():
    chain = get_version_chain(53, 53)
    assert chain == [53]


def test_unknown_version_raises():
    with pytest.raises(KeyError):
        get_version(999)


def test_all_versions_present():
    """All 62 version IDs (0-61) should be registered."""
    for i in range(62):
        v = get_version(i)
        assert v.id == i


def test_version_names_from_sdk():
    """Verify key version names match SDK header exactly."""
    assert get_version(0).name == "hk_3.0.0"
    assert get_version(39).name == "hk_2010.1.0-r1"
    assert get_version(46).name == "hk_2012.2.0-r1"
    assert get_version(53).name == "hk_2014.1.0-r1"
    assert get_version(56).name == "hk_2015.1.0-r1"
    assert get_version(61).name == "hk_2018.1.0-r1"

