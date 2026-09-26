"""Tests for Havok version registry lookups and chain-walking logic."""
import pytest
from creation_lib.havok_convert.versions import (
    get_version, get_version_by_name, get_version_chain,
)


def test_get_version_by_id_and_name_and_unknown_raises():
    v = get_version(53)
    assert v.name == "hk_2014.1.0-r1"
    assert get_version_by_name("hk_2014.1.0-r1").id == 53

    with pytest.raises(KeyError):
        get_version(999)


def test_version_chain_upgrade_downgrade_and_same():
    up = get_version_chain(46, 56)
    assert (up[0], up[-1]) == (46, 56)
    assert all(up[i] > up[i - 1] for i in range(1, len(up)))

    down = get_version_chain(56, 46)
    assert (down[0], down[-1]) == (56, 46)
    assert all(down[i] < down[i - 1] for i in range(1, len(down)))

    assert get_version_chain(53, 53) == [53]
