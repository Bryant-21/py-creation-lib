"""Mutation-pass tests for the hkxpack pyclass surface.

Validates constructors, setters, proxy lists, descriptor pyclass, and
top-level pyfunctions added in the mutation pass. Uses a synthetic packfile
(built via `creation_lib.hkxpack.write_hkx`, no real game files) wherever a
round-trip through actual bytes is needed.
"""
from __future__ import annotations

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


def _synthetic_packfile(class_names=("A", "B", "C")) -> bytes:
    hkx = HKXFile(class_version=11, contents_version="hk_2014.1.0-r1")
    hkx.objects = [
        HKXObject(name=f"#{i:04d}", class_name=name) for i, name in enumerate(class_names, 1)
    ]
    return write_hkx(hkx, DescriptorRegistry())


# --- Constructors and setters ----------------------------------------------


def test_pyclass_constructors_and_setters():
    hkx = HKXFile()
    assert hkx.class_version == 11
    assert hkx.contents_version == "hk_2014.1.0-r1"
    assert len(hkx.objects) == 0
    hkx.class_version = 12
    hkx.contents_version = "hk_2015.1.0-r1"
    assert (hkx.class_version, hkx.contents_version) == (12, "hk_2015.1.0-r1")

    obj = HKXObject(name="#0001", class_name="hkaSkeleton", schema_version=0, members=[])
    assert (obj.name, obj.class_name, len(obj.members)) == ("#0001", "hkaSkeleton", 0)
    obj.name, obj.class_name = "#0042", "hkaAnimation"
    assert (obj.name, obj.class_name) == ("#0042", "hkaAnimation")

    s = HKXStringMember(name="modelName", value="ProjectName", is_null=False)
    assert (s.name, s.value, s.is_null) == ("modelName", "ProjectName", False)
    s.value, s.is_null = "b", True
    assert (s.value, s.is_null) == ("b", True)
    assert HKXStringMember(name="x").value == ""

    d = HKXDirectMember(name="time", type=HKXType.REAL, value=1.5)
    assert d.value == 1.5
    d.value = 2.5
    assert d.value == 2.5
    assert HKXDirectMember(name="count", type=HKXType.INT32, value=42).value == 42
    assert HKXDirectMember(name="b", type=HKXType.BOOL, value=True).value is True

    a = HKXArrayMember(name="floats", subtype=HKXType.REAL)
    assert (a.subtype, len(a.contents)) == (HKXType.REAL, 0)
    a.subtype = HKXType.STRINGPTR
    assert a.subtype == HKXType.STRINGPTR

    p = HKXPointerMember(name="parent", target="#0042")
    assert p.target == "#0042"
    p.target = "#00FF"
    assert p.target == "#00FF"
    assert HKXPointerMember(name="parent").target == ""

    e = HKXEnumMember(name="kind", enum_name="AnimationType", value="HK_INTERLEAVED")
    assert e.enum_name == "AnimationType"


# --- Proxy list operations ------------------------------------------------


def test_proxy_list_append_assign_setitem_iterate():
    hkx = HKXFile()
    hkx.objects.append(HKXObject(name="#0001", class_name="hkaSkeleton"))
    assert len(hkx.objects) == 1
    assert hkx.objects[0].class_name == "hkaSkeleton"

    hkx.objects = [HKXObject(name="#0001", class_name="A"), HKXObject(name="#0002", class_name="B")]
    assert [o.class_name for o in hkx.objects] == ["A", "B"]

    hkx.objects[0] = HKXObject(name="#0003", class_name="C")
    assert hkx.objects[0].class_name == "C"


def test_proxy_object_setter_writes_through_on_synthetic_file():
    hkx = HKXFile.read_bytes(_synthetic_packfile())
    obj = hkx.objects[0]
    obj.class_name = "TestClassName"
    assert hkx.objects[0].class_name == "TestClassName"

    new_m = HKXStringMember(name="zzz_new", value="hello", is_null=False)
    obj.members.append(new_m)
    assert len(hkx.objects[0].members) == 1


def test_proxy_objects_retain_remaps_pointers_on_synthetic_file():
    """retain(predicate) drops objects AND remaps pointers — must round-trip."""
    raw = _synthetic_packfile(("A", "B", "C"))
    hkx = HKXFile.read_bytes(raw)

    hkx.objects.retain(lambda i, obj: obj.class_name != "B")
    assert [o.class_name for o in hkx.objects] == ["A", "C"]

    out = bytes(hkx.save())
    re_read = HKXFile.read_bytes(out)
    assert [o.class_name for o in re_read.objects] == ["A", "C"]


# --- Top-level pyfunctions on a synthetic packfile -------------------------


def test_load_write_detect_and_xml_round_trip(tmp_path):
    assert DescriptorRegistry().get("ThisClassDoesNotExist_xxxxx") is None

    data = _synthetic_packfile()

    hkx, reg = load_hkx_bytes(data)
    assert hkx.class_version == 11
    assert reg is not None

    out = write_hkx(hkx, reg)
    assert isinstance(out, bytes) and len(out) > 0

    fmt = detect_format(data)
    assert fmt is not None and fmt[0] == "packfile"

    xml = hkx.to_xml()
    assert xml.startswith("<?xml") and "<hkpackfile" in xml
    assert write_xml_string(hkx, reg) == xml

    out_path = tmp_path / "synthetic.xml"
    write_xml_file(hkx, reg, str(out_path))
    text = out_path.read_text(encoding="ascii")
    assert text.startswith("<?xml") and "<hkpackfile" in text
