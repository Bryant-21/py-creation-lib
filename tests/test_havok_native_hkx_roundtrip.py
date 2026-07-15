from pathlib import Path

import pytest


FO4_FIXTURES = [
    Path("resource/skeleton.hkx"),
    Path("bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx"),
]


def _native_runtime_or_skip():
    from creation_lib.havok import native_runtime

    if not native_runtime.native_available():
        pytest.skip("havok_native extension is not built")
    return native_runtime


def test_native_hkx_roundtrip_bytes_raw_preserves_committed_fo4_fixtures():
    native_runtime = _native_runtime_or_skip()

    for path in FO4_FIXTURES:
        data = path.read_bytes()
        assert native_runtime.hkx_roundtrip_bytes_raw(data) == data


def test_native_hkx_roundtrip_bytes_raw_rejects_malformed_packfile():
    native_runtime = _native_runtime_or_skip()

    malformed = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10"

    with pytest.raises(ValueError, match="packfile header"):
        native_runtime.hkx_roundtrip_bytes_raw(malformed)
