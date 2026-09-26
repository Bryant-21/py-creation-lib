from __future__ import annotations

import struct

from creation_lib.material_tools import native_runtime
from creation_lib.material_tools import _bsrefl_stringtable
from creation_lib.material_tools._bsrefl import ChunkType, find_master_string
from creation_lib.material_tools.materials_cdb import (
    BSResourceID,
    ClassDef,
    ComponentBlob,
    FieldDef,
    MaterialObject,
    MaterialsCDB,
    bethesda_crc32,
)


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


def test_native_parity_crc32_resource_id_and_master_string():
    module = native_runtime.load_native_module()
    assert module is not None
    assert callable(getattr(module, "bethesda_crc32", None))
    assert native_runtime.bethesda_crc32(b"") == 0
    assert native_runtime.bethesda_crc32(b"Bethesda") == 3937205212
    for sample in (b"", b"a", b"abc", b"materials\\weapons\\gun"):
        assert native_runtime.bethesda_crc32(sample) == bethesda_crc32(sample)

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


def test_native_parse_cdb_drops_truncated_records():
    class_name = "BSComponentDB2::DBFileIndex::ObjectInfo"
    _, offsets = _make_strt_with_strings([class_name])
    first_record = _object_info_record("materials/test/gun.mat", 1)
    partial_second_record = _object_info_record("materials/test/rifle.mat", 2)[:8]
    list_body = struct.pack("<II", offsets[class_name], 2) + first_record + partial_second_record
    data, _ = _minimal_cdb_with_strings([(ChunkType.LIST.value, list_body)], [class_name])
    assert native_runtime.parse_cdb(data)["objects"] == []

    # A truncated EdgeInfo list must not leave a dangling parent link either.
    object_class = "BSComponentDB2::DBFileIndex::ObjectInfo"
    edge_class = "BSComponentDB2::DBFileIndex::EdgeInfo"
    _, offsets2 = _make_strt_with_strings([object_class, edge_class])
    object_list_body = (
        struct.pack("<II", offsets2[object_class], 2)
        + _object_info_record("materials/test/parent.mat", 1)
        + _object_info_record("materials/test/child.mat", 2)
    )
    partial_edge_record = struct.pack("<II", 2, 1)
    edge_list_body = struct.pack("<II", offsets2[edge_class], 1) + partial_edge_record
    data2, _ = _minimal_cdb_with_strings(
        [
            (ChunkType.LIST.value, object_list_body),
            (ChunkType.LIST.value, edge_list_body),
        ],
        [object_class, edge_class],
    )
    payload2 = native_runtime.parse_cdb(data2)
    child = next(obj for obj in payload2["objects"] if obj["db_id"] == 2)
    assert child["parent_db_id"] is None


def _layerid_classdef() -> ClassDef:
    return ClassDef(
        class_name="BSMaterial::LayerID",
        class_name_index=_bsrefl_stringtable.STRING_TABLE.index("BSMaterial::LayerID"),
        class_version=1,
        class_flags=0,
        field_count=0,
    )


def _mrtexturefile_classdef() -> ClassDef:
    return ClassDef(
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


def test_native_ce2_projection_populates_texture_slots_from_normal_and_diff_payloads():
    cdb = MaterialsCDB()
    root = MaterialObject(
        persistent_id=BSResourceID(dir=1, file=2, ext=3),
        db_id=1,
        base_object_db_id=0,
        has_data=True,
    )
    root.components.append(
        ComponentBlob(class_name="BSMaterial::LayerID", is_diff=False, key=0, body=b"")
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

    cdb.class_defs["BSMaterial::LayerID"] = _layerid_classdef()
    cdb.class_defs["BSMaterial::MRTextureFile"] = _mrtexturefile_classdef()

    projected = native_runtime.project_ce2_material(cdb._to_native_payload(), 1)

    assert projected["layers"][0]["texture_set"]["diffuse"] == "textures\\test_color.dds"
    assert projected["layers"][0]["texture_set"]["normal"] == "textures\\test_normal.dds"

    # A DIFF-encoded MRTextureFile (u16 field number prefix) reads the same way.
    diff_cdb = MaterialsCDB()
    diff_root = MaterialObject(
        persistent_id=BSResourceID(dir=1, file=2, ext=3),
        db_id=1,
        base_object_db_id=0,
        has_data=True,
    )
    diff_root.components.append(
        ComponentBlob(class_name="BSMaterial::LayerID", is_diff=False, key=0, body=b"")
    )
    diff_cdb.objects_by_db_id[diff_root.db_id] = diff_root

    diff_texture_child = MaterialObject(
        persistent_id=BSResourceID(dir=4, file=5, ext=6),
        db_id=2,
        base_object_db_id=0,
        has_data=True,
        parent=diff_root,
    )
    diff_filename = b"textures\\diff_color.dds\x00"
    diff_texture_child.components.append(
        ComponentBlob(
            class_name="BSMaterial::MRTextureFile",
            is_diff=True,
            key=0,
            body=struct.pack("<HH", 0, len(diff_filename)) + diff_filename,
        )
    )
    diff_cdb.objects_by_db_id[diff_texture_child.db_id] = diff_texture_child
    diff_cdb.class_defs["BSMaterial::LayerID"] = _layerid_classdef()
    diff_cdb.class_defs["BSMaterial::MRTextureFile"] = _mrtexturefile_classdef()

    diff_projected = native_runtime.project_ce2_material(diff_cdb._to_native_payload(), 1)
    assert diff_projected["layers"][0]["texture_set"]["diffuse"] == "textures\\diff_color.dds"
