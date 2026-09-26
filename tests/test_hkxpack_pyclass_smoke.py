"""Smoke test for the read-only hkxpack pyclass MVP.

Validates that the `#[pyclass]` wrappers in `py_creation_lib/native/havok/src/python.rs`
expose the Rust HkxFile / HkxObject / HkxMember model to Python, using a synthetic
packfile built through `creation_lib.hkxpack.write_hkx` (no real game files).
"""
from __future__ import annotations

from creation_lib._native.havok_native import (
    HKXFile,
    HKXObject as NativeHKXObject,
    HKXType,
    HKXTypeFamily,
)
from creation_lib.hkxpack import DescriptorRegistry, HKXFile as WriterHKXFile, HKXObject, write_hkx


def _synthetic_packfile() -> bytes:
    hkx = WriterHKXFile(class_version=11, contents_version="hk_2014.1.0-r1")
    hkx.objects.append(HKXObject(name="#0001", class_name="hkRootLevelContainer"))
    return write_hkx(hkx, DescriptorRegistry())


def test_hkxtype_size_and_family():
    assert HKXType.REAL.size == 4
    assert HKXType.REAL.family == HKXTypeFamily.Direct
    assert HKXType.VECTOR4.size == 16
    assert HKXType.VECTOR4.family == HKXTypeFamily.Complex
    assert HKXType.STRINGPTR.family == HKXTypeFamily.String
    assert HKXType.POINTER.family == HKXTypeFamily.Pointer
    assert HKXType.ARRAY.family == HKXTypeFamily.Array
    assert HKXType.STRUCT.family == HKXTypeFamily.Object


def test_hkxfile_read_save_round_trip():
    raw = _synthetic_packfile()

    hkx = HKXFile.read_bytes(raw)
    assert hkx.class_version == 11
    assert hkx.contents_version.startswith("hk_2014")

    objs = hkx.objects
    assert len(objs) == 1
    assert len(list(objs)) == len(objs)
    obj = objs[0]
    assert isinstance(obj, NativeHKXObject)
    assert obj.class_name == "hkRootLevelContainer"

    # Read-only MVP: the snapshot stays clean, so save() echoes source bytes verbatim.
    assert hkx.save() == raw
