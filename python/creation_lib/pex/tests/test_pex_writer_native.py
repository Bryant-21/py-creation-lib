import pytest
from creation_lib.pex import native_runtime as nr

pytestmark = pytest.mark.skipif(not nr.is_available(), reason="native papyrus_core unavailable")

# Exact bytes produced by pex_writer::tests::build_minimal_bytes():
# Skyrim (game_id=1), compilation_time=1700000000, three string-table entries,
# no debug info, no user flags, no objects.
_MINIMAL_PEX = (
    b"\xde\xc0\x57\xfa"                      # magic (PEX_MAGIC LE)
    b"\x03\x09"                              # major=3, minor=9
    b"\x01\x00"                              # game_id=1 (Skyrim)
    b"\x00\xf1\x53\x65\x00\x00\x00\x00"     # compilation_time=1700000000 LE
    b"\x08\x00test.psc"                      # source_filename wstring
    b"\x06\x00tester"                        # username wstring
    b"\x02\x00pc"                            # machine_name wstring
    b"\x03\x00"                              # string_count=3
    b"\x08\x00MyScript"                      # string[0]
    b"\x04\x00None"                          # string[1]
    b"\x0f\x00ObjectReference"              # string[2]
    b"\x00"                                  # has_debug=0
    b"\x00\x00"                              # user_flags_count=0
    b"\x00\x00"                              # object_count=0
)


def test_parse_write_roundtrip_bytes(tmp_path):
    # Build a known-minimal Skyrim file via the writer round-trip: parse a real
    # file shipped in the repo's test fixtures, write it back, assert identity.
    src = tmp_path / "Mini.pex"
    src.write_bytes(_MINIMAL_PEX)
    parsed = nr.parse_pex_file_native(src)
    written = nr.write_pex_bytes_native(parsed)
    assert written == _MINIMAL_PEX
