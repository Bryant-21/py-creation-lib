import binascii
from pathlib import Path
from types import SimpleNamespace

import pytest

import creation_lib.max.cloth as cloth
from creation_lib.max.cloth import extract_cloth_document, pack_cloth_document
from creation_lib.max.bridge import (
    export_scene_document_to_nif,
    import_nif_to_scene_document,
)
from creation_lib.nif.nif_file import NifFile


FIXTURE = Path(__file__).parent / "fixtures" / "cloth" / "bathrobe_outfitm.nif"


def test_scene_document_round_trip_preserves_cloth_blob(tmp_path):
    document = import_nif_to_scene_document(str(FIXTURE))
    root_cloth = document["metadata"]["root"]["cloth"]
    assert root_cloth["version"] == 1
    assert root_cloth["blobs"][0]["raw_blob_base64"]

    output = tmp_path / "bathrobe_outfitm.nif"
    export_scene_document_to_nif(document, str(output))

    source_cloth = extract_cloth_document(
        NifFile.load(str(FIXTURE)), FIXTURE.read_bytes(), set()
    )
    output_cloth = extract_cloth_document(
        NifFile.load(str(output)), output.read_bytes(), set()
    )
    assert (
        source_cloth["blobs"][0]["raw_blob_base64"]
        == output_cloth["blobs"][0]["raw_blob_base64"]
    )


def test_export_scene_document_to_nif_keeps_existing_output_when_cloth_pack_fails(
    tmp_path, monkeypatch
):
    document = import_nif_to_scene_document(str(FIXTURE))
    document["metadata"]["root"]["cloth"] = {
        "version": 1,
        "blobs": [{"raw_blob_base64": "not base64"}],
    }
    monkeypatch.setattr(cloth, "load_havok_native_module", lambda: SimpleNamespace())
    monkeypatch.setattr(cloth, "load_nif_native_module", lambda: SimpleNamespace())
    output = tmp_path / "bathrobe_outfitm.nif"
    existing_bytes = b"existing output"
    output.write_bytes(existing_bytes)

    with pytest.raises(binascii.Error):
        export_scene_document_to_nif(document, str(output))

    assert output.read_bytes() == existing_bytes

    # Same malformed-base64 rejection at the lower-level pack_cloth_document API.
    with pytest.raises(binascii.Error):
        pack_cloth_document(b"nif", {"blobs": [{"raw_blob_base64": "not base64"}]})


def test_extract_and_pack_cloth_document_round_trip():
    nif_bytes = FIXTURE.read_bytes()
    document = extract_cloth_document(NifFile.load(str(FIXTURE)), nif_bytes, set())

    assert document["version"] == 1
    assert document["blobs"][0]["raw_blob_base64"]
    assert "summary" in document["blobs"][0]
    assert document["blobs"][0]["reverse_error"] == ""

    packed = pack_cloth_document(nif_bytes, document)
    assert packed == nif_bytes


def test_extract_cloth_document_surfaces_reverse_error(monkeypatch):
    fake_havok_native = SimpleNamespace(
        cloth_inspect_blob_json=lambda _blob: "{}",
        cloth_inspect_full_json=lambda _blob: "{}",
        cloth_reverse_to_setup=lambda _blob: '{"error": "reverse failed"}',
    )
    fake_nif_native = SimpleNamespace(cloth_extract_blob=lambda _nif_bytes: b"blob")
    monkeypatch.setattr(cloth, "load_havok_native_module", lambda: fake_havok_native)
    monkeypatch.setattr(cloth, "load_nif_native_module", lambda: fake_nif_native)

    document = extract_cloth_document(SimpleNamespace(blocks=[]), b"nif", set())

    blob = document["blobs"][0]
    assert blob["setup"] is None
    assert blob["reverse_error"] == "reverse failed"
