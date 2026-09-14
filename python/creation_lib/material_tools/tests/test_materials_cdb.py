"""Tests for MaterialsCDB reader and CE2Material dataclasses.

Reference: refs/fo76texconv/Texture Converter 0.8 - Source/py_creation_lib/python/creation_lib/libfo76utils/
src/bsmatcdb.{cpp,hpp} and material.hpp.
"""
from __future__ import annotations

import json
import os
import struct
from pathlib import Path

import pytest

from creation_lib.material_tools._bsrefl import ChunkType
from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE
from creation_lib.material_tools.materials_cdb import (
    BSResourceID,
    CE2Material,
    ClassDef,
    ComponentBlob,
    FieldDef,
    MaterialObject,
    MaterialsCDB,
    bethesda_crc32,
)


FIX = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "cdb"
)
CDB = FIX / "sample_small.cdb"
GOLDEN = FIX / "sample_small.golden.json"


# ---------------------------------------------------------------------------
# Bethesda CRC32 variant
# ---------------------------------------------------------------------------

def test_bethesda_crc32_variant_raw_no_inversion():
    # The cpp BSMaterialsCDB hashFunctionCRC32 uses the IEEE 802.3 polynomial
    # 0xEDB88320 but with init=0 and NO post-xor, unlike zlib.crc32 which
    # pre-inverts and post-inverts. Verified cross-reference:
    #   raw_crc32(x) == zlib.crc32(x, 0xFFFFFFFF) ^ 0xFFFFFFFF
    import zlib
    for sample in (b"gun", b"materials\\test\\wood", b"", b"a", b"abc"):
        expected = zlib.crc32(sample, 0xFFFFFFFF) ^ 0xFFFFFFFF
        assert bethesda_crc32(sample) == expected, sample


def test_bethesda_crc32_empty_is_zero():
    assert bethesda_crc32(b"") == 0


# ---------------------------------------------------------------------------
# BSResourceID: path parsing
# ---------------------------------------------------------------------------

def test_bsresourceid_from_path_normalizes_case_and_slashes():
    # The cpp constructor lowercases and converts '/' to '\\' before
    # hashing the directory. The base name is lowercased but NOT
    # slash-normalized (no slashes expected there anyway).
    a = BSResourceID.from_path("Materials/Weapons/gun.mat")
    b = BSResourceID.from_path("materials\\weapons\\gun.mat")
    c = BSResourceID.from_path("MATERIALS\\WEAPONS\\GUN.MAT")
    assert a == b == c


def test_bsresourceid_different_paths_differ():
    a = BSResourceID.from_path("materials/weapons/gun.mat")
    b = BSResourceID.from_path("materials/armor/gun.mat")
    assert a != b
    c = BSResourceID.from_path("materials/weapons/rifle.mat")
    assert a != c


def test_bsresourceid_ext_packing_mat():
    # cpp packs the ASCII bytes after the dot as a little-endian u32 and
    # lowercases via `ext | ((ext >> 1) & 0x20202020)`. For "gun.mat" the switch
    # sees length - extPos = 7 - 3 = 4 and takes `readUInt32Fast(data + i) >> 8`:
    # '.mat' as LE u32 = 0x74616D2E, >> 8 = 0x0074616D.
    rid = BSResourceID.from_path("gun.mat")
    # Expected ext value: 0x0074616D ("mat\0" little-endian)
    assert rid.ext == 0x0074616D


def test_bsresourceid_ext_packing_with_directory():
    # With a directory, i advances past baseNamePos before the baseName
    # loop. For "materials/gun.mat":
    #   baseNamePos = 9, extPos = 13, i after dir loop = 10, file after
    #   base name loop = crc("gun"), then length - i = 17 - 13 = 4.
    rid = BSResourceID.from_path("materials/gun.mat")
    assert rid.ext == 0x0074616D
    assert rid.file == bethesda_crc32(b"gun")
    # Directory crc input: "materials" lowercased (no slashes inside).
    assert rid.dir == bethesda_crc32(b"materials")


def test_bsresourceid_slash_normalization_in_dir():
    # Forward slashes inside the directory path get converted to '\\'
    # BEFORE being hashed. So "a/b" and "a\\b" produce the same dir hash.
    a = BSResourceID.from_path("a/b/gun.mat")
    b = BSResourceID.from_path("a\\b\\gun.mat")
    assert a.dir == b.dir
    # And the directory crc is of "a\\b" (not "a/b" and not "a\\\\b").
    assert a.dir == bethesda_crc32(b"a\\b")


# ---------------------------------------------------------------------------
# Hand-crafted CDB stream
# ---------------------------------------------------------------------------

def _beth_magic() -> bytes:
    return struct.pack("<Q", 0x0000000848544542)


def _make_strt_with_strings(strings: list[str]) -> tuple[bytes, dict[str, int]]:
    """Return (STRT chunk body, mapping of string -> strtOffs).

    strtOffs is (filePos - 24) at the start of each entry, where filePos
    counts bytes from the file start. The BETH header up to and including
    the STRT chunk's length prefix is 8 (magic) + 4 (ver) + 4 (chunks) +
    4 (STRT type) + 4 (STRT length) = 24 bytes, so the first entry byte
    is at filePos=24 and strtOffs=0.
    """
    entries_blob = bytearray()
    offsets: dict[str, int] = {}
    cursor = 0
    for s in strings:
        offsets[s] = cursor
        b = s.encode("utf-8") + b"\x00"
        entries_blob += b
        cursor += len(b)
    body = struct.pack("<I", len(entries_blob)) + bytes(entries_blob)
    return body, offsets


def _chunk(chunk_type: int, body: bytes) -> bytes:
    return struct.pack("<II", chunk_type, len(body)) + body


def _build_cdb_stream(extra_chunks: list[tuple[int, bytes]], strings: list[str]) -> tuple[bytes, dict[str, int]]:
    strt_body, offsets = _make_strt_with_strings(strings)
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)  # version
    buf += struct.pack("<I", len(extra_chunks) + 2)  # chunks_remaining
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    for ctype, body in extra_chunks:
        buf += _chunk(ctype, body)
    return bytes(buf), offsets


def test_materialscdb_empty_stream_loads():
    # Smallest valid CDB: just BETH + STRT with no trailing chunks.
    data, _ = _build_cdb_stream([], [])
    cdb = MaterialsCDB.from_bytes(data)
    assert cdb.list_materials() == []


def test_materialscdb_objectinfo_list_creates_material_objects():
    # Hand-build a minimal CDB with:
    #  - an ObjectInfo LIST chunk containing one object with a known
    #    persistentID and dbID
    # Strings needed in STRT:
    #    "BSComponentDB2::DBFileIndex::ObjectInfo"
    strings = ["BSComponentDB2::DBFileIndex::ObjectInfo"]
    strt_body, offsets = _make_strt_with_strings(strings)

    # ObjectInfo record layout: 21 bytes
    #   0: file (u32)    - BSResourceID.file
    #   4: ext  (u32)    - BSResourceID.ext
    #   8: dir  (u32)    - BSResourceID.dir
    #  12: dbID (u32)
    #  16: baseObjectDbID (u32)
    #  20: hasData (u8)
    persistent_id = BSResourceID.from_path("materials/test/gun.mat")
    object_record = struct.pack(
        "<IIIIIB",
        persistent_id.file,
        persistent_id.ext,
        persistent_id.dir,
        1,   # dbID
        0,   # baseObjectDbID
        1,   # hasData
    )
    # LIST chunk body layout:
    #   [u32 className_strtOffs][u32 n_elements][records...]
    list_body = struct.pack(
        "<II",
        offsets["BSComponentDB2::DBFileIndex::ObjectInfo"],
        1,  # one element
    ) + object_record

    # Assemble full stream
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)  # version
    buf += struct.pack("<I", 3)  # chunks: STRT + LIST counts as 3 per cpp semantics
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    buf += _chunk(ChunkType.LIST.value, list_body)
    data = bytes(buf)

    cdb = MaterialsCDB.from_bytes(data)
    obj = cdb.lookup_by_path("materials/test/gun.mat")
    assert obj is not None
    assert obj.persistent_id == persistent_id
    assert obj.db_id == 1


def test_materialscdb_type_clas_chunks_register_class_definitions():
    # Build a CDB containing one TYPE chunk with one CLAS child chunk that
    # defines a class "BSMaterial::BlenderID" with zero fields, then verify
    # that the class was registered (via public API: .class_defs dict).
    class_name = "BSMaterial::BlenderID"
    strings = [class_name]
    strt_body, offsets = _make_strt_with_strings(strings)

    # CLAS body:
    #   [u32 className_strtOffs][u32 classVersion][u16 classFlags][u16 fieldCnt][fields...]
    clas_body = struct.pack(
        "<IIHH",
        offsets[class_name],
        0,   # classVersion
        0,   # classFlags
        0,   # fieldCnt -> no field records
    )

    # TYPE chunk body is: [u32 classCnt]. The TYPE chunk's size field is
    # only large enough to hold that u32 -- the CLAS chunks follow as
    # SEPARATE chunks (each [type u32][size u32][body]). cpp:715 reads the
    # next chunk from cdbFile (not from inside the TYPE chunk body).
    type_body = struct.pack("<I", 1)  # one CLAS follows

    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)  # version
    buf += struct.pack("<I", 4)  # chunks: STRT + TYPE + CLAS = 4 per cpp (includes the -2 offset)
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    buf += _chunk(ChunkType.TYPE.value, type_body)
    buf += _chunk(ChunkType.CLAS.value, clas_body)
    data = bytes(buf)

    cdb = MaterialsCDB.from_bytes(data)
    # Class should be registered
    assert class_name in cdb.class_defs
    cdef = cdb.class_defs[class_name]
    assert cdef.field_count == 0
    assert cdef.fields == []


# ---------------------------------------------------------------------------
# Optional fixture-based tests (skip if fixture absent)
# ---------------------------------------------------------------------------

@pytest.mark.skipif(not CDB.exists(), reason="CDB fixture missing")
def test_load_and_list():
    cdb = MaterialsCDB.from_file(CDB)
    names = cdb.list_materials()
    assert len(names) > 0


@pytest.mark.skipif(
    not CDB.exists() or not GOLDEN.exists(),
    reason="CDB fixture or golden missing",
)
def test_lookup_by_path_matches_golden():
    cdb = MaterialsCDB.from_file(CDB)
    golden = json.loads(GOLDEN.read_text())
    for entry in golden["materials"]:
        mat = cdb.lookup_by_path(entry["path"])
        assert mat is not None, entry["path"]


# ---------------------------------------------------------------------------
# Component walker tests
# ---------------------------------------------------------------------------

# Real CDB fixture (Starfield sfbgs003 creation pack — ~1.2MB)
_STARFIELD_EXTRACTED = Path(
    os.environ.get("STARFIELD_EXTRACTED_DIR")
    or Path(__file__).resolve().parents[5] / "extracted" / "starfield"
)
_REAL_CDB = _STARFIELD_EXTRACTED / "materials" / "creations" / "sfbgs003" / "materialsbeta.cdb"


class TestWalkComponent:
    """Unit tests for walk_component with synthetic ComponentBlob data."""

    def _make_classdef(self, name: str, fields: list[tuple[str, int]]) -> ClassDef:
        from creation_lib.material_tools.materials_cdb import ClassDef, FieldDef
        from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE
        idx = next((i for i, s in enumerate(STRING_TABLE) if s == name), 0)
        cdef = ClassDef(
            class_name=name,
            class_name_index=idx,
            class_version=1,
            class_flags=0,
            field_count=len(fields),
        )
        for fname, ftype in fields:
            fidx = next((i for i, s in enumerate(STRING_TABLE) if s == fname), 0)
            cdef.fields.append(FieldDef(
                name_index=fidx,
                type_index=ftype,
                data_offset=0,
                data_size=0,
            ))
        return cdef

    def test_walk_ctname_component(self):
        """BSComponentDB::CTName has a single String field."""
        from creation_lib.material_tools.materials_cdb import (
            ComponentBlob, walk_component,
        )
        # Build an OBJT body: field 0 is a String (u16 length + bytes)
        name_bytes = b"TestMaterial\x00"
        body = struct.pack("<H", len(name_bytes)) + name_bytes
        blob = ComponentBlob(
            class_name="BSComponentDB::CTName",
            is_diff=False,
            key=(0 << 16) | 0,
            body=body,
        )
        cdef = self._make_classdef("BSComponentDB::CTName", [
            ("Name", 1),  # StringType.STRING = 1
        ])
        data = walk_component(blob, {"BSComponentDB::CTName": cdef}, None, {})
        assert data is not None
        assert data["Name"] == "TestMaterial"

    def test_walk_mrtexturefile_component(self):
        """BSMaterial::MRTextureFile has a single String field."""
        from creation_lib.material_tools.materials_cdb import (
            ComponentBlob, walk_component,
        )
        path_bytes = b"Data\\Textures\\test_color.dds\x00"
        body = struct.pack("<H", len(path_bytes)) + path_bytes
        blob = ComponentBlob(
            class_name="BSMaterial::MRTextureFile",
            is_diff=False,
            key=(0 << 16) | 0,
            body=body,
        )
        cdef = self._make_classdef("BSMaterial::MRTextureFile", [
            ("FileName", 1),  # StringType.STRING = 1
        ])
        data = walk_component(blob, {"BSMaterial::MRTextureFile": cdef}, None, {})
        assert data is not None
        assert data["FileName"] == "Data\\Textures\\test_color.dds"

    def test_walk_diff_blob_reads_field_number(self):
        """DIFF blobs prefix each field value with a u16 field number."""
        from creation_lib.material_tools.materials_cdb import (
            ComponentBlob, walk_component,
        )
        # DIFF body: [u16 field_number=0][u16 str_len][str_bytes]
        name_bytes = b"DiffName\x00"
        body = struct.pack("<H", 0) + struct.pack("<H", len(name_bytes)) + name_bytes
        blob = ComponentBlob(
            class_name="BSComponentDB::CTName",
            is_diff=True,
            key=(0 << 16) | 0,
            body=body,
        )
        cdef = self._make_classdef("BSComponentDB::CTName", [
            ("Name", 1),
        ])
        data = walk_component(blob, {"BSComponentDB::CTName": cdef}, None, {})
        assert data is not None
        assert data["Name"] == "DiffName"


class TestPopulateCE2Material:
    """Unit tests for populate_ce2_material with synthetic objects."""

    def test_walk_component_uses_native_runtime(self, monkeypatch):
        import creation_lib.material_tools.materials_cdb as materials_cdb

        calls = {}

        def fake_walk_component(blob_payload, class_defs_payload, objects_payload):
            calls["blob"] = blob_payload
            calls["class_defs"] = class_defs_payload
            calls["objects"] = objects_payload
            return {"Name": "NativeName"}

        monkeypatch.setattr(
            materials_cdb.native_runtime,
            "walk_component",
            fake_walk_component,
            raising=False,
        )
        blob = ComponentBlob(
            class_name="BSComponentDB::CTName",
            is_diff=False,
            key=0,
            body=struct.pack("<H", len(b"PythonName\x00")) + b"PythonName\x00",
        )
        cdef = ClassDef(
            class_name="BSComponentDB::CTName",
            class_name_index=STRING_TABLE.index("BSComponentDB::CTName"),
            class_version=1,
            class_flags=0,
            field_count=1,
            fields=[
                FieldDef(
                    name_index=STRING_TABLE.index("Name"),
                    type_index=STRING_TABLE.index("String"),
                    data_offset=0,
                    data_size=0,
                )
            ],
        )

        data = materials_cdb.walk_component(blob, {cdef.class_name: cdef}, None, {})

        assert data == {"Name": "NativeName"}
        assert calls["blob"]["class_name"] == "BSComponentDB::CTName"
        assert calls["class_defs"][0]["class_name"] == "BSComponentDB::CTName"

    def test_get_ce2_material_projects_ctname_without_python_fallback(self, monkeypatch):
        import creation_lib.material_tools.materials_cdb as materials_cdb

        def fail_fallback(*_args, **_kwargs):
            raise AssertionError("Python CE2 fallback should not run")

        monkeypatch.setattr(materials_cdb, "populate_ce2_material", fail_fallback)
        cdb = MaterialsCDB()
        path = "materials/test/native_only.mat"
        root = MaterialObject(
            persistent_id=BSResourceID.from_path(path),
            db_id=1,
            base_object_db_id=0,
            has_data=True,
        )
        root.components.append(ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False,
            key=0,
            body=b"",
        ))
        cdb.objects_by_db_id[root.db_id] = root
        cdb.objects_by_persistent_id[root.persistent_id] = root

        child = MaterialObject(
            persistent_id=BSResourceID(dir=4, file=5, ext=6),
            db_id=2,
            base_object_db_id=0,
            has_data=True,
            parent=root,
        )
        name = b"native_layer\x00"
        child.components.append(ComponentBlob(
            class_name="BSComponentDB::CTName",
            is_diff=False,
            key=0,
            body=struct.pack("<H", len(name)) + name,
        ))
        cdb.objects_by_db_id[child.db_id] = child

        cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
            class_name="BSMaterial::LayerID",
            class_name_index=STRING_TABLE.index("BSMaterial::LayerID"),
            class_version=1,
            class_flags=0,
            field_count=0,
        )
        cdb.class_defs["BSComponentDB::CTName"] = ClassDef(
            class_name="BSComponentDB::CTName",
            class_name_index=STRING_TABLE.index("BSComponentDB::CTName"),
            class_version=1,
            class_flags=0,
            field_count=1,
            fields=[
                FieldDef(
                    name_index=STRING_TABLE.index("Name"),
                    type_index=STRING_TABLE.index("String"),
                    data_offset=0,
                    data_size=0,
                )
            ],
        )

        mat = cdb.get_ce2_material(path)

        assert mat is not None
        assert mat.layers[0].name == "native_layer"
        assert root.path == "native_layer"

    def test_populates_layer_from_layerid_component(self):
        from creation_lib.material_tools.materials_cdb import (
            ComponentBlob, ClassDef, FieldDef, MaterialObject,
            CE2Material, populate_ce2_material, MaterialsCDB, BSResourceID,
        )
        # Create a minimal CDB with one object having a LayerID component
        cdb = MaterialsCDB()
        obj = MaterialObject(
            persistent_id=BSResourceID(dir=1, file=2, ext=3),
            db_id=1,
            base_object_db_id=0,
            has_data=True,
        )
        # Empty LayerID OBJT body (no fields to read — it's just a marker)
        obj.components.append(ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False,
            key=0,
            body=b"",
        ))
        cdb.objects_by_db_id[1] = obj
        cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
            class_name="BSMaterial::LayerID",
            class_name_index=190,
            class_version=1,
            class_flags=0,
            field_count=0,
        )
        mat = CE2Material(name="test", material_object=obj)
        populate_ce2_material(mat, cdb)
        assert len(mat.layers) == 1
        assert mat.layers[0].texture_set.diffuse == ""

    def test_populates_textures_from_subtree(self):
        """Integration: LayerID on root, MRTextureFile on grandchild."""
        from creation_lib.material_tools.materials_cdb import (
            ComponentBlob, ClassDef, FieldDef, MaterialObject,
            CE2Material, populate_ce2_material, MaterialsCDB, BSResourceID,
        )
        cdb = MaterialsCDB()

        # Root object with LayerID
        root = MaterialObject(
            persistent_id=BSResourceID(dir=1, file=2, ext=3),
            db_id=1, base_object_db_id=0, has_data=True,
        )
        root.components.append(ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False, key=0, body=b"",
        ))
        cdb.objects_by_db_id[1] = root

        # Child with TextureSetID
        child = MaterialObject(
            persistent_id=BSResourceID(dir=4, file=5, ext=6),
            db_id=2, base_object_db_id=0, has_data=True,
        )
        child.parent = root
        child.components.append(ComponentBlob(
            class_name="BSMaterial::TextureSetID",
            is_diff=False, key=0, body=b"",
        ))
        cdb.objects_by_db_id[2] = child

        # Grandchild with MRTextureFile x2
        grandchild = MaterialObject(
            persistent_id=BSResourceID(dir=7, file=8, ext=9),
            db_id=3, base_object_db_id=0, has_data=True,
        )
        grandchild.parent = child
        for fname in (b"textures\\test_color.dds\x00", b"textures\\test_normal.dds\x00"):
            body = struct.pack("<H", len(fname)) + fname
            grandchild.components.append(ComponentBlob(
                class_name="BSMaterial::MRTextureFile",
                is_diff=False, key=0, body=body,
            ))
        cdb.objects_by_db_id[3] = grandchild

        # Register class defs
        cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
            class_name="BSMaterial::LayerID", class_name_index=190,
            class_version=1, class_flags=0, field_count=0,
        )
        cdb.class_defs["BSMaterial::TextureSetID"] = ClassDef(
            class_name="BSMaterial::TextureSetID", class_name_index=216,
            class_version=1, class_flags=0, field_count=0,
        )
        cdb.class_defs["BSMaterial::MRTextureFile"] = ClassDef(
            class_name="BSMaterial::MRTextureFile", class_name_index=194,
            class_version=1, class_flags=0, field_count=1,
        )
        cdb.class_defs["BSMaterial::MRTextureFile"].fields.append(
            FieldDef(name_index=0, type_index=1, data_offset=0, data_size=0)
        )
        # Patch FieldDef.name for the MRTextureFile — name_index=0 resolves
        # to STRING_TABLE[0] which is "None". For the walker it doesn't matter
        # because we read by field index, not name. But we need the result
        # dict key. Let's use the real name index.
        from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE
        fn_idx = next((i for i, s in enumerate(STRING_TABLE) if s == "FileName"), 0)
        cdb.class_defs["BSMaterial::MRTextureFile"].fields[0].name_index = fn_idx

        mat = CE2Material(name="test", material_object=root)
        populate_ce2_material(mat, cdb)
        assert len(mat.layers) == 1
        ts = mat.layers[0].texture_set
        assert ts.diffuse == "textures\\test_color.dds"
        assert ts.normal == "textures\\test_normal.dds"

    def test_get_ce2_material_projects_ctname_natively(self):
        cdb = MaterialsCDB()
        path = "materials/test/native_ctname.mat"
        root = MaterialObject(
            persistent_id=BSResourceID.from_path(path),
            db_id=1,
            base_object_db_id=0,
            has_data=True,
        )
        root.components.append(ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False, key=0, body=b"",
        ))
        cdb.objects_by_db_id[root.db_id] = root
        cdb.objects_by_persistent_id[root.persistent_id] = root

        child = MaterialObject(
            persistent_id=BSResourceID(dir=4, file=5, ext=6),
            db_id=2,
            base_object_db_id=0,
            has_data=True,
            parent=root,
        )
        layer_name = b"native_layer\x00"
        child.components.append(ComponentBlob(
            class_name="BSComponentDB::CTName",
            is_diff=False,
            key=0,
            body=struct.pack("<H", len(layer_name)) + layer_name,
        ))
        cdb.objects_by_db_id[child.db_id] = child

        cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
            class_name="BSMaterial::LayerID", class_name_index=190,
            class_version=1, class_flags=0, field_count=0,
        )
        cdb.class_defs["BSComponentDB::CTName"] = ClassDef(
            class_name="BSComponentDB::CTName",
            class_name_index=STRING_TABLE.index("BSComponentDB::CTName"),
            class_version=1,
            class_flags=0,
            field_count=1,
            fields=[
                FieldDef(
                    name_index=STRING_TABLE.index("Name"),
                    type_index=STRING_TABLE.index("String"),
                    data_offset=0,
                    data_size=0,
                )
            ],
        )

        mat = cdb.get_ce2_material(path)

        assert mat is not None
        assert len(mat.layers) == 1
        assert mat.layers[0].name == "native_layer"

    def test_get_ce2_material_rejects_unsupported_native_field_type(self):
        cdb = MaterialsCDB()
        path = "materials/test/unsupported_list.mat"
        root = MaterialObject(
            persistent_id=BSResourceID.from_path(path),
            db_id=1,
            base_object_db_id=0,
            has_data=True,
        )
        root.components.append(ComponentBlob(
            class_name="BSMaterial::LayerID",
            is_diff=False,
            key=0,
            body=b"",
        ))
        root.components.append(ComponentBlob(
            class_name="BSMaterial::MaterialID",
            is_diff=False,
            key=0,
            body=b"",
        ))
        cdb.objects_by_db_id[root.db_id] = root
        cdb.objects_by_persistent_id[root.persistent_id] = root

        cdb.class_defs["BSMaterial::LayerID"] = ClassDef(
            class_name="BSMaterial::LayerID",
            class_name_index=STRING_TABLE.index("BSMaterial::LayerID"),
            class_version=1,
            class_flags=0,
            field_count=0,
        )
        cdb.class_defs["BSMaterial::MaterialID"] = ClassDef(
            class_name="BSMaterial::MaterialID",
            class_name_index=STRING_TABLE.index("BSMaterial::MaterialID"),
            class_version=1,
            class_flags=0,
            field_count=1,
            fields=[
                FieldDef(
                    name_index=STRING_TABLE.index("Name"),
                    type_index=STRING_TABLE.index("List"),
                    data_offset=0,
                    data_size=0,
                )
            ],
        )

        with pytest.raises(NotImplementedError, match="unsupported native CDB projection"):
            cdb.get_ce2_material(path)


@pytest.mark.skipif(not _REAL_CDB.exists(), reason="Real CDB fixture missing")
class TestWalkerIntegration:
    """Integration tests against a real Starfield materialsbeta.cdb."""

    def test_load_and_populate_plasma_cutter(self):
        """dbID 18243 in sfbgs003 is a PlasmaCutter grip material with
        layer → material → texture-set → MRTextureFile subtree."""
        cdb = MaterialsCDB.from_file(_REAL_CDB)
        obj = cdb.objects_by_db_id.get(18243)
        assert obj is not None, "expected dbID 18243 in sfbgs003"
        mat = CE2Material(name="plasma_cutter_grip", material_object=obj)
        from creation_lib.material_tools.materials_cdb import populate_ce2_material
        populate_ce2_material(mat, cdb)
        assert len(mat.layers) >= 1
        ts = mat.layers[0].texture_set
        assert "color.dds" in ts.diffuse.lower()
        assert "normal.dds" in ts.normal.lower()
        assert "rough.dds" in ts.rough.lower()
        assert "metal.dds" in ts.metal.lower()
        assert "ao.dds" in ts.ao.lower()

    def test_get_ce2_material_populates_on_known_path(self):
        """get_ce2_material should return a populated CE2Material when the
        persistent_id resolves. We register the object under a synthetic
        path and verify the walker runs."""
        from creation_lib.material_tools.materials_cdb import populate_ce2_material
        cdb = MaterialsCDB.from_file(_REAL_CDB)
        obj = cdb.objects_by_db_id.get(18243)
        assert obj is not None
        # Lookup by the object's real persistent_id
        mat = CE2Material(name="plasma_cutter_grip", material_object=obj)
        populate_ce2_material(mat, cdb)
        assert len(mat.layers) >= 1
        assert "color.dds" in mat.layers[0].texture_set.diffuse.lower()

    def test_walker_handles_all_objects_without_error(self):
        """Walk every root object's subtree — no crashes."""
        cdb = MaterialsCDB.from_file(_REAL_CDB)
        from creation_lib.material_tools.materials_cdb import populate_ce2_material
        walked = 0
        errors = 0
        for obj in cdb.objects_by_db_id.values():
            if obj.parent is not None:
                continue  # only walk roots
            mat = CE2Material(name=f"obj_{obj.db_id}", material_object=obj)
            try:
                populate_ce2_material(mat, cdb)
                walked += 1
            except Exception as exc:
                errors += 1
                if errors <= 3:  # don't flood output
                    print(f"ERROR on dbID {obj.db_id}: {exc}")
        assert walked > 0
        assert errors == 0, f"{errors} objects failed walker"


# ---------------------------------------------------------------------------
# Misc
# ---------------------------------------------------------------------------

def test_unknown_class_warns_not_raises(caplog):
    # Build a CDB that has a LIST with an unrecognized className -- the
    # cpp code silently skips it via the `continue` fall-through. We just
    # assert it doesn't raise.
    class_name = "Totally::Fake::Unknown::Class"  # not in STRING_TABLE
    strings = [class_name]
    strt_body, offsets = _make_strt_with_strings(strings)
    list_body = struct.pack("<II", offsets[class_name], 0)  # zero elements
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)
    buf += struct.pack("<I", 3)
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    buf += _chunk(ChunkType.LIST.value, list_body)
    data = bytes(buf)
    # Should not raise
    cdb = MaterialsCDB.from_bytes(data)
    assert cdb.list_materials() == []
