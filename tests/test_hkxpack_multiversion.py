"""Multi-version writer behavior — must use HKXFile.contents_version, not hardcoded FO4.

The other version-aware checks (descriptor registry version dispatch, pyclass
constructors) live in tests/test_hkxpack_pyclass_{smoke,mutation}.py and the
native cargo suite under py_creation_lib/native/havok/tests/.
"""
import json

from creation_lib.hkxpack import DescriptorRegistry, HKXFile, HKXObject, write_hkx
from creation_lib.hkxpack.native_runtime import _require_native


def _inspect(data: bytes) -> dict:
    return json.loads(_require_native().hkx_inspect_packfile(data))


def test_write_uses_file_version_for_header():
    """Writer must honor hkx_file.contents_version, not hardcode FO4 (hk_2014)."""
    hkx = HKXFile(class_version=11, contents_version="hk_2015.1.0-r1")
    hkx.objects.append(HKXObject(name="#0001", class_name="hkRootLevelContainer"))
    raw = write_hkx(hkx, DescriptorRegistry())
    inspected = _inspect(raw)
    assert "2015" in inspected["header"]["version_name"]
