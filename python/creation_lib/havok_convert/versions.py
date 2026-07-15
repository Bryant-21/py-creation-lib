"""Havok version registry -- IDs, names, and chain navigation.

All 62 version IDs (0-61) from hkHavokVersions.h with human-readable names
matching the Havok SDK's version string format (e.g. "hk_2014.1.0-r1").
"""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class HavokVersion:
    id: int
    name: str


# All 62 versions from refs/hk2018_1_0_r1/Source/Common/Compat/hkHavokVersions.h
# Names derived from SDK macro names following the Havok version string convention.
_VERSIONS: list[HavokVersion] = [
    HavokVersion(0, "hk_3.0.0"),             # HK_HAVOK_VERSION_300
    HavokVersion(1, "hk_3.1.0"),             # HK_HAVOK_VERSION_310
    HavokVersion(2, "hk_3.2.0"),             # HK_HAVOK_VERSION_320
    HavokVersion(3, "hk_3.3.0-a2"),          # HK_HAVOK_VERSION_330a2
    HavokVersion(4, "hk_3.3.0-b1"),          # HK_HAVOK_VERSION_330b1
    HavokVersion(5, "hk_3.3.0-b2"),          # HK_HAVOK_VERSION_330b2
    HavokVersion(6, "hk_3.3.0-r1"),          # HK_HAVOK_VERSION_330r1
    HavokVersion(7, "hk_4.0.0-b1"),          # HK_HAVOK_VERSION_400b1
    HavokVersion(8, "hk_4.0.0-b2"),          # HK_HAVOK_VERSION_400b2
    HavokVersion(9, "hk_4.0.0-r1"),          # HK_HAVOK_VERSION_400r1
    HavokVersion(10, "hk_4.0.2-r1"),         # HK_HAVOK_VERSION_402r1
    HavokVersion(11, "hk_4.0.3-r1"),         # HK_HAVOK_VERSION_403r1
    HavokVersion(12, "hk_4.1.0-b1"),         # HK_HAVOK_VERSION_410b1
    HavokVersion(13, "hk_4.1.0-r1"),         # HK_HAVOK_VERSION_410r1
    HavokVersion(14, "hk_4.5.0-b1"),         # HK_HAVOK_VERSION_450b1
    HavokVersion(15, "hk_4.5.0-r1"),         # HK_HAVOK_VERSION_450r1
    HavokVersion(16, "hk_4.5.1-r1"),         # HK_HAVOK_VERSION_451r1
    HavokVersion(17, "hk_4.5.2-r1"),         # HK_HAVOK_VERSION_452r1
    HavokVersion(18, "hk_4.6.0-b1"),         # HK_HAVOK_VERSION_460b1
    HavokVersion(19, "hk_4.6.0-b2"),         # HK_HAVOK_VERSION_460b2
    HavokVersion(20, "hk_4.6.0-r1"),         # HK_HAVOK_VERSION_460r1
    HavokVersion(21, "hk_4.6.1-r1"),         # HK_HAVOK_VERSION_461r1
    HavokVersion(22, "hk_5.0.0-b1"),         # HK_HAVOK_VERSION_500b1
    HavokVersion(23, "hk_5.0.0-r1"),         # HK_HAVOK_VERSION_500r1
    HavokVersion(24, "hk_5.1.0-r1"),         # HK_HAVOK_VERSION_510r1
    HavokVersion(25, "hk_5.5.0-b1"),         # HK_HAVOK_VERSION_550b1
    HavokVersion(26, "hk_5.5.0-b2"),         # HK_HAVOK_VERSION_550b2
    HavokVersion(27, "hk_5.5.0-r1"),         # HK_HAVOK_VERSION_550r1
    HavokVersion(28, "hk_6.0.0-b1"),         # HK_HAVOK_VERSION_600b1
    HavokVersion(29, "hk_6.0.0-b2"),         # HK_HAVOK_VERSION_600b2
    HavokVersion(30, "hk_6.0.0-r1"),         # HK_HAVOK_VERSION_600r1
    HavokVersion(31, "hk_6.1.0-r1"),         # HK_HAVOK_VERSION_610r1
    HavokVersion(32, "hk_6.5.0-b1"),         # HK_HAVOK_VERSION_650b1
    HavokVersion(33, "hk_6.5.0-r1"),         # HK_HAVOK_VERSION_650r1
    HavokVersion(34, "hk_6.6.0-b1"),         # HK_HAVOK_VERSION_660b1
    HavokVersion(35, "hk_6.6.0-r1"),         # HK_HAVOK_VERSION_660r1
    HavokVersion(36, "hk_7.0.0-b1"),         # HK_HAVOK_VERSION_700b1
    HavokVersion(37, "hk_7.0.0-r1"),         # HK_HAVOK_VERSION_700r1
    HavokVersion(38, "hk_7.1.0-r1"),         # HK_HAVOK_VERSION_710r1
    HavokVersion(39, "hk_2010.1.0-r1"),      # HK_HAVOK_VERSION_201010r1
    HavokVersion(40, "hk_2010.2.0-r1"),      # HK_HAVOK_VERSION_201020r1
    HavokVersion(41, "hk_2011.1.0-r1"),      # HK_HAVOK_VERSION_201110r1
    HavokVersion(42, "hk_2011.2.0-r1"),      # HK_HAVOK_VERSION_201120r1
    HavokVersion(43, "hk_2011.3.0-r1"),      # HK_HAVOK_VERSION_201130r1
    HavokVersion(44, "hk_2011.3.1-r1"),      # HK_HAVOK_VERSION_201131r1
    HavokVersion(45, "hk_2012.1.0-r1"),      # HK_HAVOK_VERSION_201210r1
    HavokVersion(46, "hk_2012.2.0-r1"),      # HK_HAVOK_VERSION_201220r1
    HavokVersion(47, "hk_2012.2.1-r1"),      # HK_HAVOK_VERSION_201221r1
    HavokVersion(48, "hk_2013.1.0-r1"),      # HK_HAVOK_VERSION_201310r1
    HavokVersion(49, "hk_2013.1.1-r1"),      # HK_HAVOK_VERSION_201311r1
    HavokVersion(50, "hk_2013.2.0-r1"),      # HK_HAVOK_VERSION_201320r1
    HavokVersion(51, "hk_2013.2.5-r1"),      # HK_HAVOK_VERSION_201325r1
    HavokVersion(52, "hk_2013.3.0-r1"),      # HK_HAVOK_VERSION_201330r1
    HavokVersion(53, "hk_2014.1.0-r1"),      # HK_HAVOK_VERSION_201410r1
    HavokVersion(54, "hk_2014.1.0-r2"),      # HK_HAVOK_VERSION_201410r2
    HavokVersion(55, "hk_2014.2.0-r1"),      # HK_HAVOK_VERSION_201420r1
    HavokVersion(56, "hk_2015.1.0-r1"),      # HK_HAVOK_VERSION_201510r1
    HavokVersion(57, "hk_2016.1.0-r1"),      # HK_HAVOK_VERSION_201610r1
    HavokVersion(58, "hk_2016.2.0-r1"),      # HK_HAVOK_VERSION_201620r1
    HavokVersion(59, "hk_2017.1.0-r1"),      # HK_HAVOK_VERSION_201710r1
    HavokVersion(60, "hk_2017.2.0-r1"),      # HK_HAVOK_VERSION_201720r1
    HavokVersion(61, "hk_2018.1.0-r1"),      # HK_HAVOK_VERSION_Current
]

_BY_ID: dict[int, HavokVersion] = {v.id: v for v in _VERSIONS}
_BY_NAME: dict[str, HavokVersion] = {v.name: v for v in _VERSIONS}
_SORTED_IDS: list[int] = sorted(_BY_ID.keys())

# Game shortcuts
SKYRIM_SE = _BY_ID[46]
FO4 = _BY_ID[53]
FO76 = _BY_ID[56]


def get_version(version_id: int) -> HavokVersion:
    """Get version by numeric ID. Raises KeyError if not found."""
    return _BY_ID[version_id]


def get_version_by_name(name: str) -> HavokVersion:
    """Get version by name string. Raises KeyError if not found."""
    return _BY_NAME[name]


def get_version_chain(src: int, dst: int) -> list[int]:
    """Get ordered list of version IDs from src to dst (inclusive)."""
    if src == dst:
        return [src]
    if src < dst:
        return [v for v in _SORTED_IDS if src <= v <= dst]
    else:
        return [v for v in reversed(_SORTED_IDS) if dst <= v <= src]
