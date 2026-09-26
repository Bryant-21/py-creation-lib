import pytest


def _native_runtime_or_skip():
    from creation_lib.havok import native_runtime

    if not native_runtime.native_available():
        pytest.skip("havok_native extension is not built")
    return native_runtime


def test_native_hkx_roundtrip_bytes_raw_preserves_synthetic_packfile():
    from creation_lib.hkxpack import DescriptorRegistry, HKXFile, HKXObject, write_hkx

    native_runtime = _native_runtime_or_skip()

    hkx = HKXFile(class_version=11, contents_version="hk_2014.1.0-r1")
    hkx.objects.append(HKXObject(name="#0001", class_name="hkRootLevelContainer"))
    data = write_hkx(hkx, DescriptorRegistry())

    assert native_runtime.hkx_roundtrip_bytes_raw(data) == data


def test_native_hkx_roundtrip_bytes_raw_rejects_malformed_packfile():
    native_runtime = _native_runtime_or_skip()

    malformed = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10"

    with pytest.raises(ValueError, match="packfile header"):
        native_runtime.hkx_roundtrip_bytes_raw(malformed)
