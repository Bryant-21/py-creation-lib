"""Smoke test for the read-only hkxpack pyclass MVP.

Validates that the new `#[pyclass]` wrappers in `py_creation_lib/native/havok/src/python.rs`
expose the Rust HkxFile / HkxObject / HkxMember model to Python with the
shape callers in `py_creation_lib/python/creation_lib/hkxpack/` will need to migrate against.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib._native.havok_native import (
    HKXArrayMember,
    HKXDirectMember,
    HKXEnumMember,
    HKXFile,
    HKXObject,
    HKXPointerMember,
    HKXStringMember,
    HKXType,
    HKXTypeFamily,
)


# A small vanilla FO4 packfile shipped under extracted/fo4/. AlienProject.hkx
# is ~944 bytes and parses cleanly through the v11 packfile reader.
_FIXTURE = Path("extracted/fo4/Meshes/Actors/Alien/AlienProject.hkx")


def test_hkxtype_size_and_family():
    assert HKXType.REAL.size == 4
    assert HKXType.REAL.family == HKXTypeFamily.Direct
    assert HKXType.VECTOR4.size == 16
    assert HKXType.VECTOR4.family == HKXTypeFamily.Complex
    assert HKXType.STRINGPTR.family == HKXTypeFamily.String
    assert HKXType.POINTER.family == HKXTypeFamily.Pointer
    assert HKXType.ARRAY.family == HKXTypeFamily.Array
    assert HKXType.STRUCT.family == HKXTypeFamily.Object


def test_hkxfile_read_and_objects():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    assert hkx.class_version == 11
    assert hkx.contents_version.startswith("hk_2014")

    objs = hkx.objects
    # Mutation pass: objects is now an HKXObjectList proxy, not a plain list.
    # Verify it still implements the list protocol (len, iter, indexing).
    assert len(objs) > 0
    assert len(list(objs)) == len(objs)

    print()
    print(f"=== {_FIXTURE.name}: {len(objs)} objects ===")
    for obj in list(objs)[:3]:
        assert isinstance(obj, HKXObject)
        members = obj.members
        print(f"  {obj.name!r}  class={obj.class_name!r}  members={len(members)}")


def test_member_isinstance_dispatch():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    variants = (
        HKXDirectMember,
        HKXArrayMember,
        HKXPointerMember,
        HKXStringMember,
        HKXEnumMember,
    )
    saw_any_member = False
    seen_classes: set[type] = set()
    for obj in hkx.objects:
        for m in obj.members:
            saw_any_member = True
            assert isinstance(m, variants), (
                f"member {m!r} on {obj.class_name} is not one of "
                f"the expected variant pyclasses"
            )
            seen_classes.add(type(m))
            # Every member must expose a name attribute.
            assert isinstance(m.name, str)
    assert saw_any_member, "no members surfaced from the test fixture"
    print(f"=== variant classes seen: {sorted(c.__name__ for c in seen_classes)} ===")


def test_save_round_trip_non_empty():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    out = hkx.save()
    assert isinstance(out, bytes)
    assert len(out) > 0
    # Read-only MVP: the snapshot stays clean, so save() echoes source bytes
    # verbatim.
    assert out == _FIXTURE.read_bytes()


def test_read_bytes_matches_read():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    data = _FIXTURE.read_bytes()
    a = HKXFile.read_bytes(data)
    b = HKXFile.read(str(_FIXTURE))
    assert a.class_version == b.class_version
    assert a.contents_version == b.contents_version
    assert len(a.objects) == len(b.objects)


def test_member_isinstance_dispatch_iterates():
    """Iterate proxy lists via the __iter__ protocol added in mutation pass."""
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    seen_classes = set()
    for obj in hkx.objects:
        for m in obj.members:
            seen_classes.add(type(m).__name__)
    assert seen_classes, "no members surfaced"
    print(f"variant classes: {sorted(seen_classes)}")
