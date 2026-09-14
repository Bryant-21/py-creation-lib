from pathlib import Path
import struct

import pytest

from creation_lib.ba2 import native_runtime as archive


@pytest.mark.parametrize("kind", ["fo76", "fo4", "starfield", "tes4", "fo3", "sse"])
@pytest.mark.parametrize("compressed", [False, True])
@pytest.mark.parametrize("workers", [1, 4])
def test_bulk_extraction_matches_single_reads_and_overwrites(tmp_path, kind, compressed, workers):
    source = tmp_path / "source"
    source.mkdir()
    payloads = {
        "assets/shared/large.bin": bytes(range(256)) * 2048,
        "assets/shared/small.bin": b"small",
        "assets/shared/empty.bin": b"",
        "assets/other/member.bin": b"other" * 1024,
    }
    entries = []
    for index, (name, payload) in enumerate(payloads.items()):
        path = source / str(index)
        path.write_bytes(payload)
        entries.append((str(path), name))
    packed = tmp_path / "input.archive"
    archive.pack_archive_entries(entries, str(packed), kind, compress=compressed, jobs=2)
    assert archive.archive_entry_count(str(packed)) == len(payloads)
    assert archive.archive_info(str(packed))["file_count"] == len(payloads)

    output = tmp_path / "output"
    for name in payloads:
        dest = output / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(b"stale" * 200_000)
    events = []
    assert archive.extract_archive(str(packed), str(output), workers=workers, progress=events.append) == len(payloads)
    assert len(list(output.rglob("*.bin"))) == len(payloads)
    for name, payload in payloads.items():
        assert (output / name).read_bytes() == payload == archive.extract_one(str(packed), name)
    assert max(event["completed"] for event in events) == len(payloads)


@pytest.mark.parametrize("version", [1, 2, 3, 7, 8])
def test_ba2_count_reads_only_validated_header(tmp_path, version):
    header = struct.pack("<4sI4sIQ", b"BTDX", version, b"GNRL", 12345, 36)
    if version in (2, 3):
        header += bytes(8)
    if version == 3:
        header += struct.pack("<I", 3)
    path = tmp_path / "header.ba2"
    path.write_bytes(header)
    assert archive.archive_entry_count(str(path)) == 12345
    path.write_bytes(header[:-1])
    with pytest.raises(RuntimeError):
        archive.archive_entry_count(str(path))


@pytest.mark.parametrize("version", [103, 104, 105])
def test_bsa_count_reads_only_validated_header(tmp_path, version):
    header = struct.pack("<4s8I", b"BSA\0", version, 36, 3, 50, 12345, 0, 0, 0)
    path = tmp_path / "header.bsa"
    path.write_bytes(header)
    assert archive.archive_entry_count(str(path)) == 12345
    path.write_bytes(header[:-1])
    with pytest.raises(RuntimeError):
        archive.archive_entry_count(str(path))


@pytest.mark.parametrize("header", [
    b"invalid header",
    struct.pack("<4sI4sIQ", b"BTDX", 99, b"GNRL", 1, 24),
    struct.pack("<4sI4sIQ", b"BTDX", 1, b"NOPE", 1, 24),
    struct.pack("<4s8I", b"BSA\0", 105, 37, 3, 1, 1, 0, 0, 0),
])
def test_header_count_rejects_invalid_headers(tmp_path, header):
    path = tmp_path / "invalid.archive"
    path.write_bytes(header)
    with pytest.raises(RuntimeError):
        archive.archive_entry_count(str(path))


def test_cancel_stops_sequential_extraction(tmp_path):
    source = tmp_path / "payload"
    source.write_bytes(b"payload")
    packed = tmp_path / "input.ba2"
    archive.pack_archive_entries(
        [(str(source), f"assets/{index}.bin") for index in range(10)],
        str(packed), "fo76",
    )
    output = tmp_path / "output"
    with pytest.raises(RuntimeError, match="extraction cancelled"):
        archive.extract_archive(str(packed), str(output), workers=1, progress=lambda _event: False)
    assert len(list(output.rglob("*.bin"))) == 1


def test_bad_compressed_member_does_not_truncate_existing_output(tmp_path):
    source = tmp_path / "payload"
    source.write_bytes(b"compressible" * 1000)
    packed = tmp_path / "input.ba2"
    archive.pack_archive_entries([(str(source), "assets/member.bin")], str(packed), "fo76")
    data = bytearray(packed.read_bytes())
    offset, packed_size = struct.unpack_from("<QI", data, 24 + 16)
    assert packed_size > 0
    data[offset:offset + packed_size] = bytes(packed_size)
    packed.write_bytes(data)
    output = tmp_path / "output"
    dest = output / "assets/member.bin"
    dest.parent.mkdir(parents=True)
    dest.write_bytes(b"keep existing on decode failure")
    with pytest.raises(RuntimeError, match="decompression"):
        archive.extract_archive(str(packed), str(output), workers=1)
    assert dest.read_bytes() == b"keep existing on decode failure"


@pytest.mark.parametrize("kind", ["fo76dds", "starfielddds"])
@pytest.mark.parametrize("workers", [1, 4])
def test_texture_bulk_extraction_preserves_headers_mips_and_cubemaps(tmp_path, kind, workers):
    fixtures = Path(__file__).resolve().parents[1] / "native/bsarchive/data"
    entries = [
        (str(fixtures / "fo4_chunk_test/test.dds"), "textures/chunks.dds"),
        (str(fixtures / "fo4_cubemap_test/blacksky_e.dds"), "textures/cubemap.dds"),
        (str(fixtures / "fo4_dx9_test/dx9.dds"), "textures/legacy.dds"),
    ]
    packed = tmp_path / "textures.ba2"
    archive.pack_archive_entries(entries, str(packed), kind, compress=True, jobs=2)
    output = tmp_path / "output"
    assert archive.extract_archive(str(packed), str(output), workers=workers) == len(entries)
    for _source, name in entries:
        assert (output / name).read_bytes() == archive.extract_one(str(packed), name)
