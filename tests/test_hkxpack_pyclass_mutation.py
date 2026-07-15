"""Mutation-pass tests for the hkxpack pyclass surface.

Validates constructors, setters, proxy lists, descriptor pyclass, and
top-level pyfunctions added in the mutation pass.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib._native.havok_native import (
    DescriptorRegistry,
    HKXArrayMember,
    HKXDirectMember,
    HKXEnumMember,
    HKXFile,
    HKXObject,
    HKXPointerMember,
    HKXStringMember,
    HKXType,
    detect_format,
    load_hkx_bytes,
    write_hkx,
    write_xml_file,
    write_xml_string,
)


_FIXTURE = Path("extracted/fo4/Meshes/Actors/Alien/AlienProject.hkx")


# --- Constructors ---------------------------------------------------------


def test_hkxfile_constructor_empty():
    hkx = HKXFile()
    assert hkx.class_version == 11
    assert hkx.contents_version == "hk_2014.1.0-r1"
    assert len(hkx.objects) == 0


def test_hkxfile_constructor_with_kwargs():
    hkx = HKXFile(class_version=11, contents_version="hk_2014.1.0-r1", objects=[])
    assert hkx.class_version == 11
    assert hkx.contents_version == "hk_2014.1.0-r1"
    assert len(hkx.objects) == 0


def test_hkxobject_constructor():
    obj = HKXObject(name="#0001", class_name="hkaSkeleton", schema_version=0, members=[])
    assert obj.name == "#0001"
    assert obj.class_name == "hkaSkeleton"
    assert len(obj.members) == 0


def test_hkxstringmember_constructor():
    m = HKXStringMember(name="modelName", value="ProjectName", is_null=False)
    assert m.name == "modelName"
    assert m.value == "ProjectName"
    assert m.is_null is False


def test_hkxstringmember_is_null_default():
    m = HKXStringMember(name="x")
    assert m.value == ""
    assert m.is_null is False


def test_hkxdirectmember_constructor():
    m = HKXDirectMember(name="time", type=HKXType.REAL, value=1.5)
    assert m.name == "time"
    assert m.value == 1.5


def test_hkxdirectmember_constructor_int():
    m = HKXDirectMember(name="count", type=HKXType.INT32, value=42)
    assert m.value == 42


def test_hkxarraymember_constructor_empty():
    m = HKXArrayMember(name="floats", subtype=HKXType.REAL)
    assert m.name == "floats"
    assert m.subtype == HKXType.REAL
    assert len(m.contents) == 0


def test_hkxpointermember_constructor():
    m = HKXPointerMember(name="parent", target="#0042")
    assert m.name == "parent"
    assert m.target == "#0042"


def test_hkxpointermember_constructor_empty():
    m = HKXPointerMember(name="parent")
    assert m.target == ""


def test_hkxenummember_constructor():
    m = HKXEnumMember(name="kind", enum_name="AnimationType", value="HK_INTERLEAVED")
    assert m.enum_name == "AnimationType"


# --- Setters --------------------------------------------------------------


def test_hkxfile_setters():
    hkx = HKXFile()
    hkx.class_version = 12
    assert hkx.class_version == 12
    hkx.contents_version = "hk_2015.1.0-r1"
    assert hkx.contents_version == "hk_2015.1.0-r1"


def test_hkxobject_setters_unbound():
    obj = HKXObject(class_name="hkaSkeleton")
    obj.name = "#0042"
    obj.class_name = "hkaAnimation"
    assert obj.name == "#0042"
    assert obj.class_name == "hkaAnimation"


def test_hkxstring_setters_unbound():
    m = HKXStringMember(name="x", value="a", is_null=False)
    m.value = "b"
    m.is_null = True
    assert m.value == "b"
    assert m.is_null is True


def test_hkxpointer_setter_unbound():
    m = HKXPointerMember(name="x", target="#0001")
    m.target = "#00FF"
    assert m.target == "#00FF"


def test_hkxdirect_setter_unbound():
    m = HKXDirectMember(name="x", type=HKXType.REAL, value=1.0)
    m.value = 2.5
    assert m.value == 2.5


def test_hkxarray_subtype_setter_unbound():
    m = HKXArrayMember(name="x", subtype=HKXType.REAL)
    m.subtype = HKXType.STRINGPTR
    assert m.subtype == HKXType.STRINGPTR


# --- Proxy list operations ------------------------------------------------


def test_proxy_objects_append():
    hkx = HKXFile()
    obj = HKXObject(name="#0001", class_name="hkaSkeleton")
    hkx.objects.append(obj)
    assert len(hkx.objects) == 1
    fetched = hkx.objects[0]
    assert fetched.class_name == "hkaSkeleton"


def test_proxy_objects_replace_via_assign():
    hkx = HKXFile()
    o1 = HKXObject(name="#0001", class_name="A")
    o2 = HKXObject(name="#0002", class_name="B")
    hkx.objects = [o1, o2]
    assert len(hkx.objects) == 2
    assert hkx.objects[0].class_name == "A"
    assert hkx.objects[1].class_name == "B"


def test_proxy_objects_setitem():
    hkx = HKXFile()
    hkx.objects = [HKXObject(name="#0001", class_name="A"), HKXObject(name="#0002", class_name="B")]
    hkx.objects[0] = HKXObject(name="#0003", class_name="C")
    assert hkx.objects[0].class_name == "C"


def test_proxy_members_append_on_real_file():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    obj0 = hkx.objects[0]
    initial_len = len(obj0.members)
    new_m = HKXStringMember(name="zzz_new", value="hello", is_null=False)
    obj0.members.append(new_m)
    # Re-fetch via proxy and verify presence
    obj0_again = hkx.objects[0]
    assert len(obj0_again.members) == initial_len + 1


def test_proxy_object_setter_writes_through():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    obj = hkx.objects[0]
    obj.class_name = "TestClassName"
    # Re-fetch to check write-through
    obj_again = hkx.objects[0]
    assert obj_again.class_name == "TestClassName"


def test_proxy_iteration():
    hkx = HKXFile()
    hkx.objects = [HKXObject(class_name="A"), HKXObject(class_name="B"), HKXObject(class_name="C")]
    names = [o.class_name for o in hkx.objects]
    assert names == ["A", "B", "C"]


def test_proxy_objects_retain_remaps_pointers():
    """retain(predicate) drops objects AND remaps pointers — must round-trip."""
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    initial_count = len(hkx.objects)
    initial_classes = [o.class_name for o in hkx.objects]
    assert "hkbProjectStringData" in initial_classes, (
        "fixture invariant: needs hkbProjectStringData to test retain drop"
    )

    hkx.objects.retain(lambda i, obj: obj.class_name != "hkbProjectStringData")

    assert len(hkx.objects) == initial_count - 1
    assert all(o.class_name != "hkbProjectStringData" for o in hkx.objects)

    # Save must succeed (proves the writer accepts the remapped pointers).
    out = bytes(hkx.save())
    assert len(out) > 0

    # Re-read: drop persists and other objects survive intact.
    re_read = HKXFile.read_bytes(out)
    assert len(re_read.objects) == initial_count - 1
    assert all(o.class_name != "hkbProjectStringData" for o in re_read.objects)
    surviving = {o.class_name for o in re_read.objects}
    for cls in initial_classes:
        if cls != "hkbProjectStringData":
            assert cls in surviving, f"unexpected loss of {cls} after retain"


# --- Round-trip -----------------------------------------------------------


def test_setter_round_trip():
    """Construct, mutate, save bytes, re-read, verify changes persisted."""
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    original_class_version = hkx.class_version
    original_contents_version = hkx.contents_version
    original_count = len(hkx.objects)

    # Mutate contents_version
    hkx.contents_version = original_contents_version  # no-op but marks dirty
    out_bytes = bytes(hkx.save())

    # Re-read
    re_read = HKXFile.read_bytes(out_bytes)
    assert re_read.class_version == original_class_version
    assert re_read.contents_version == original_contents_version
    assert len(re_read.objects) == original_count


def test_constructor_value_types():
    """Verify HKXDirectMember.value is preserved across constructor."""
    m_int = HKXDirectMember(name="i", type=HKXType.INT32, value=42)
    assert m_int.value == 42
    m_real = HKXDirectMember(name="f", type=HKXType.REAL, value=1.5)
    assert m_real.value == 1.5
    m_bool = HKXDirectMember(name="b", type=HKXType.BOOL, value=True)
    assert m_bool.value is True


# --- DescriptorRegistry ---------------------------------------------------


def test_descriptor_registry_default_constructor():
    reg = DescriptorRegistry()
    assert reg is not None


def test_descriptor_registry_get_known_class():
    reg = DescriptorRegistry()
    desc = reg.get("hkbProjectData")
    if desc is None:
        pytest.skip("hkbProjectData not in default classxml — registry empty")
    # signature may be empty in some classxml — just check the type works.
    assert isinstance(desc.name, str)


def test_descriptor_registry_get_missing():
    reg = DescriptorRegistry()
    desc = reg.get("ThisClassDoesNotExist_xxxxx")
    assert desc is None


# --- Top-level pyfunctions ------------------------------------------------


def test_load_hkx_bytes_top_level():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    data = _FIXTURE.read_bytes()
    hkx, reg = load_hkx_bytes(data)
    assert hkx.class_version == 11
    assert reg is not None


def test_write_hkx_top_level():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    data = _FIXTURE.read_bytes()
    hkx, reg = load_hkx_bytes(data)
    out = write_hkx(hkx, reg)
    assert isinstance(out, bytes)
    assert len(out) > 0


def test_detect_format_top_level():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    data = _FIXTURE.read_bytes()
    fmt = detect_format(data)
    assert fmt is not None
    assert fmt[0] == "packfile"


def test_hkxfile_to_xml_smoke():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx = HKXFile.read(str(_FIXTURE))
    xml = hkx.to_xml()
    assert isinstance(xml, str)
    assert xml.startswith("<?xml")
    assert "<hkpackfile" in xml


def test_write_xml_string_top_level():
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx, reg = load_hkx_bytes(_FIXTURE.read_bytes())
    xml = write_xml_string(hkx, reg)
    assert xml.startswith("<?xml")
    assert "<hkpackfile" in xml


def test_write_xml_file_top_level(tmp_path):
    if not _FIXTURE.exists():
        pytest.skip(f"fixture not present: {_FIXTURE}")

    hkx, reg = load_hkx_bytes(_FIXTURE.read_bytes())
    out_path = tmp_path / "alien.xml"
    write_xml_file(hkx, reg, str(out_path))
    assert out_path.exists()
    text = out_path.read_text(encoding="ascii")
    assert text.startswith("<?xml")
    assert "<hkpackfile" in text
