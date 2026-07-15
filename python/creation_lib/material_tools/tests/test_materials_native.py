from __future__ import annotations

import io
import struct
from dataclasses import asdict
from pathlib import Path

from creation_lib.material_tools import native_runtime
from creation_lib.material_tools import _bsrefl_stringtable
from creation_lib.material_tools._bsrefl import ChunkType, find_master_string
from creation_lib.material_tools.bgem_bin import read_bgem
from creation_lib.material_tools.bgsm_bin import read_bgsm
from creation_lib.material_tools.materials_cdb import (
    BSResourceID,
    ClassDef,
    ComponentBlob,
    FieldDef,
    MaterialObject,
    MaterialsCDB,
    bethesda_crc32,
)

FIXTURE_DIR = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "materials"
)


def _normalize_payload(value):
    if hasattr(value, "__dataclass_fields__"):
        return {k: _normalize_payload(v) for k, v in asdict(value).items()}
    if isinstance(value, tuple):
        return [_normalize_payload(v) for v in value]
    if isinstance(value, list):
        return [_normalize_payload(v) for v in value]
    if isinstance(value, dict):
        return {k: _normalize_payload(v) for k, v in value.items()}
    return value


def _beth_magic() -> bytes:
    return struct.pack("<Q", 0x0000000848544542)


def _make_strt_with_strings(strings: list[str]) -> tuple[bytes, dict[str, int]]:
    entries = bytearray()
    offsets: dict[str, int] = {}
    for value in strings:
        offsets[value] = len(entries)
        entries.extend(value.encode("utf-8"))
        entries.append(0)
    return struct.pack("<I", len(entries)) + bytes(entries), offsets


def _chunk(chunk_type: int, body: bytes) -> bytes:
    return struct.pack("<II", chunk_type, len(body)) + body


def _minimal_bsrefl(
    extra_chunks: list[tuple[int, bytes]],
    strings: list[str] | None = None,
) -> bytes:
    strt_body, _ = _make_strt_with_strings(strings or [])
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)
    buf += struct.pack("<I", len(extra_chunks) + 2)
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    for chunk_type, body in extra_chunks:
        buf += _chunk(chunk_type, body)
    return bytes(buf)


def _minimal_cdb_with_strings(
    extra_chunks: list[tuple[int, bytes]],
    strings: list[str],
) -> tuple[bytes, dict[str, int]]:
    strt_body, offsets = _make_strt_with_strings(strings)
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)
    buf += struct.pack("<I", len(extra_chunks) + 2)
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    for chunk_type, body in extra_chunks:
        buf += _chunk(chunk_type, body)
    return bytes(buf), offsets


def _object_info_record(path: str, db_id: int) -> bytes:
    rid = BSResourceID.from_path(path)
    return struct.pack(
        "<IIIIIB",
        rid.file,
        rid.ext,
        rid.dir,
        db_id,
        0,
        1,
    )


def test_materials_native_module_loads():
    module = native_runtime.load_native_module()
    assert module is not None
    assert callable(getattr(module, "bethesda_crc32", None))


def test_bethesda_crc32_matches_known_values():
    assert native_runtime.bethesda_crc32(b"") == 0
    assert native_runtime.bethesda_crc32(b"Bethesda") == 3937205212


def test_materials_native_crc_matches_python():
    for sample in (b"", b"a", b"abc", b"materials\\weapons\\gun"):
        assert native_runtime.bethesda_crc32(sample) == bethesda_crc32(sample)


def test_materials_native_resource_id_matches_python():
    paths = [
        "Materials/Weapons/gun.mat",
        "materials\\weapons\\gun.mat",
        "MATERIALS\\WEAPONS\\GUN.MAT",
        "gun.mat",
        "materials/é/gün.mat",
        "materials/gun.måt",
    ]
    for path in paths:
        expected = BSResourceID.from_path(path)
        actual = native_runtime.resource_id_from_path(path)
        assert actual == {
            "dir": expected.dir,
            "file": expected.file,
            "ext": expected.ext,
        }


def test_native_find_master_string_matches_python():
    assert native_runtime.find_master_string("BSResource::ID") == find_master_string(
        "BSResource::ID"
    )
    assert native_runtime.find_master_string("this::symbol::does::not::exist") == -1


def test_native_bsrefl_summary_matches_synthetic_stream():
    data = _minimal_bsrefl(
        [
            (ChunkType.TYPE.value, b"hi"),
            (ChunkType.LIST.value, b""),
        ]
    )

    actual = native_runtime.inspect_bsrefl(data)

    assert actual == {
        "chunks_remaining": 2,
        "chunks": [
            {"type": ChunkType.TYPE.value, "size": 2},
            {"type": ChunkType.LIST.value, "size": 0},
        ],
    }


def test_native_parse_cdb_object_info_matches_python_lookup():
    class_name = "BSComponentDB2::DBFileIndex::ObjectInfo"
    rid = BSResourceID.from_path("materials/test/gun.mat")
    object_record = _object_info_record("materials/test/gun.mat", 1)
    _, offsets = _make_strt_with_strings([class_name])
    list_body = struct.pack("<II", offsets[class_name], 1) + object_record
    data, _ = _minimal_cdb_with_strings([(ChunkType.LIST.value, list_body)], [class_name])

    payload = native_runtime.parse_cdb(data)

    first_object = payload["objects"][0]
    assert first_object["db_id"] == 1
    assert first_object["persistent_id"] == {
        "dir": rid.dir,
        "file": rid.file,
        "ext": rid.ext,
    }


def test_native_parse_cdb_drops_truncated_object_info_list():
    class_name = "BSComponentDB2::DBFileIndex::ObjectInfo"
    _, offsets = _make_strt_with_strings([class_name])
    first_record = _object_info_record("materials/test/gun.mat", 1)
    partial_second_record = _object_info_record("materials/test/rifle.mat", 2)[:8]
    list_body = struct.pack("<II", offsets[class_name], 2) + first_record + partial_second_record
    data, _ = _minimal_cdb_with_strings([(ChunkType.LIST.value, list_body)], [class_name])

    payload = native_runtime.parse_cdb(data)

    assert payload["objects"] == []


def test_native_parse_cdb_drops_truncated_edge_info_list_parent_link():
    object_class = "BSComponentDB2::DBFileIndex::ObjectInfo"
    edge_class = "BSComponentDB2::DBFileIndex::EdgeInfo"
    _, offsets = _make_strt_with_strings([object_class, edge_class])
    object_list_body = (
        struct.pack("<II", offsets[object_class], 2)
        + _object_info_record("materials/test/parent.mat", 1)
        + _object_info_record("materials/test/child.mat", 2)
    )
    partial_edge_record = struct.pack("<II", 2, 1)
    edge_list_body = struct.pack("<II", offsets[edge_class], 1) + partial_edge_record
    data, _ = _minimal_cdb_with_strings(
        [
            (ChunkType.LIST.value, object_list_body),
            (ChunkType.LIST.value, edge_list_body),
        ],
        [object_class, edge_class],
    )

    payload = native_runtime.parse_cdb(data)

    child = next(obj for obj in payload["objects"] if obj["db_id"] == 2)
    assert child["parent_db_id"] is None


def test_native_ce2_projection_populates_texture_slots_from_payload():
    cdb = MaterialsCDB()
    root = MaterialObject(
        persistent_id=BSResourceID(dir=1, file=2, ext=3),
        db_id=1,
        base_object_db_id=0,
        has_data=True,
    )
    root.components.append(
        ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False,
            key=0,
            body=b"",
        )
    )
    cdb.objects_by_db_id[root.db_id] = root

    child = MaterialObject(
        persistent_id=BSResourceID(dir=4, file=5, ext=6),
        db_id=2,
        base_object_db_id=0,
        has_data=True,
        parent=root,
    )
    cdb.objects_by_db_id[child.db_id] = child

    texture_child = MaterialObject(
        persistent_id=BSResourceID(dir=7, file=8, ext=9),
        db_id=3,
        base_object_db_id=0,
        has_data=True,
        parent=child,
    )
    for filename in (
        b"textures\\test_color.dds\x00",
        b"textures\\test_normal.dds\x00",
    ):
        texture_child.components.append(
            ComponentBlob(
                class_name="BSMaterial::MRTextureFile",
                is_diff=False,
                key=0,
                body=struct.pack("<H", len(filename)) + filename,
            )
        )
    cdb.objects_by_db_id[texture_child.db_id] = texture_child

    cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
        class_name="BSMaterial::LayerID",
        class_name_index=_bsrefl_stringtable.STRING_TABLE.index("BSMaterial::LayerID"),
        class_version=1,
        class_flags=0,
        field_count=0,
    )
    cdb.class_defs["BSMaterial::MRTextureFile"] = ClassDef(
        class_name="BSMaterial::MRTextureFile",
        class_name_index=_bsrefl_stringtable.STRING_TABLE.index("BSMaterial::MRTextureFile"),
        class_version=1,
        class_flags=0,
        field_count=1,
        fields=[
            FieldDef(
                name_index=_bsrefl_stringtable.STRING_TABLE.index("FileName"),
                type_index=_bsrefl_stringtable.STRING_TABLE.index("String"),
                data_offset=0,
                data_size=0,
            )
        ],
    )

    payload = cdb._to_native_payload()
    projected = native_runtime.project_ce2_material(payload, 1)

    assert projected["layers"][0]["texture_set"]["diffuse"] == "textures\\test_color.dds"
    assert projected["layers"][0]["texture_set"]["normal"] == "textures\\test_normal.dds"


def test_native_ce2_projection_reads_diff_mrtexturefile_payload():
    cdb = MaterialsCDB()
    root = MaterialObject(
        persistent_id=BSResourceID(dir=1, file=2, ext=3),
        db_id=1,
        base_object_db_id=0,
        has_data=True,
    )
    root.components.append(
        ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False,
            key=0,
            body=b"",
        )
    )
    cdb.objects_by_db_id[root.db_id] = root

    texture_child = MaterialObject(
        persistent_id=BSResourceID(dir=4, file=5, ext=6),
        db_id=2,
        base_object_db_id=0,
        has_data=True,
        parent=root,
    )
    filename = b"textures\\diff_color.dds\x00"
    texture_child.components.append(
        ComponentBlob(
            class_name="BSMaterial::MRTextureFile",
            is_diff=True,
            key=0,
            body=struct.pack("<HH", 0, len(filename)) + filename,
        )
    )
    cdb.objects_by_db_id[texture_child.db_id] = texture_child

    cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
        class_name="BSMaterial::LayerID",
        class_name_index=_bsrefl_stringtable.STRING_TABLE.index("BSMaterial::LayerID"),
        class_version=1,
        class_flags=0,
        field_count=0,
    )
    cdb.class_defs["BSMaterial::MRTextureFile"] = ClassDef(
        class_name="BSMaterial::MRTextureFile",
        class_name_index=_bsrefl_stringtable.STRING_TABLE.index("BSMaterial::MRTextureFile"),
        class_version=1,
        class_flags=0,
        field_count=1,
        fields=[
            FieldDef(
                name_index=_bsrefl_stringtable.STRING_TABLE.index("FileName"),
                type_index=_bsrefl_stringtable.STRING_TABLE.index("String"),
                data_offset=0,
                data_size=0,
            )
        ],
    )

    projected = native_runtime.project_ce2_material(cdb._to_native_payload(), 1)

    assert projected["layers"][0]["texture_set"]["diffuse"] == "textures\\diff_color.dds"


def test_native_bgsm_parse_matches_python_fixture():
    data = (FIXTURE_DIR / "sample_v22.bgsm").read_bytes()

    expected = _normalize_payload(read_bgsm(io.BytesIO(data)))
    actual = _normalize_payload(native_runtime.parse_bgsm(data))

    assert actual == expected


def test_native_bgsm_write_round_trips_fixture():
    data = (FIXTURE_DIR / "sample_v22.bgsm").read_bytes()

    payload = native_runtime.parse_bgsm(data)

    assert native_runtime.write_bgsm(payload) == data


def test_native_bgsm_writer_null_terminates_generated_texture_paths():
    data = (FIXTURE_DIR / "sample_v22.bgsm").read_bytes()
    payload = native_runtime.parse_bgsm(data)
    payload["DiffuseTexture"] = payload["DiffuseTexture"].rstrip("\x00")
    payload["NormalTexture"] = payload["NormalTexture"].rstrip("\x00")

    written = native_runtime.write_bgsm(payload)

    for field in ("DiffuseTexture", "NormalTexture"):
        encoded = payload[field].encode("utf-8")
        offset = written.index(encoded)
        encoded_len = int.from_bytes(written[offset - 4 : offset], "little")
        assert encoded_len == len(encoded) + 1
        assert written[offset + len(encoded)] == 0


def test_native_bgem_parse_matches_python_fixture():
    data = (FIXTURE_DIR / "sample_v22.bgem").read_bytes()

    expected = _normalize_payload(read_bgem(io.BytesIO(data)))
    actual = _normalize_payload(native_runtime.parse_bgem(data))

    assert actual == expected


def test_native_bgem_glass_write_round_trips_fixture():
    data = (FIXTURE_DIR / "sample_v22_glass.bgem").read_bytes()

    payload = native_runtime.parse_bgem(data)

    assert payload["GlassEnabled"] is True
    assert native_runtime.write_bgem(payload) == data


def test_public_bgsm_reader_uses_native_roundtrip_contract():
    original = (FIXTURE_DIR / "sample_v22.bgsm").read_bytes()
    data = read_bgsm(io.BytesIO(original))
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == original


def test_public_bgem_reader_uses_native_roundtrip_contract():
    original = (FIXTURE_DIR / "sample_v22_glass.bgem").read_bytes()
    data = read_bgem(io.BytesIO(original))
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == original


def test_public_bgsm_reader_and_writer_delegate_to_native(monkeypatch):
    payload = native_runtime.parse_bgsm((FIXTURE_DIR / "sample_v22.bgsm").read_bytes())
    calls = {}

    def fake_parse(raw):
        calls["parse_raw"] = raw
        return payload

    def fake_write(native_payload):
        calls["write_payload"] = native_payload
        return b"native-bgsm"

    monkeypatch.setattr(native_runtime, "parse_bgsm", fake_parse)
    monkeypatch.setattr(native_runtime, "write_bgsm", fake_write)

    data = read_bgsm(io.BytesIO(b"raw-bgsm"))

    assert calls["parse_raw"] == b"raw-bgsm"
    assert isinstance(data.SpecularColor, tuple)
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == b"native-bgsm"
    assert calls["write_payload"]["header"] == asdict(data.header)


def test_public_bgem_reader_and_writer_delegate_to_native(monkeypatch):
    payload = native_runtime.parse_bgem(
        (FIXTURE_DIR / "sample_v22_glass.bgem").read_bytes()
    )
    calls = {}

    def fake_parse(raw):
        calls["parse_raw"] = raw
        return payload

    def fake_write(native_payload):
        calls["write_payload"] = native_payload
        return b"native-bgem"

    monkeypatch.setattr(native_runtime, "parse_bgem", fake_parse)
    monkeypatch.setattr(native_runtime, "write_bgem", fake_write)

    data = read_bgem(io.BytesIO(b"raw-bgem"))

    assert calls["parse_raw"] == b"raw-bgem"
    assert isinstance(data.BaseColor, tuple)
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == b"native-bgem"
    assert calls["write_payload"]["header"] == asdict(data.header)
