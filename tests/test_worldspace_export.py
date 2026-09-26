import struct

from creation_lib.esp.model import Group, Record, Subrecord


def _zstring(value: str) -> bytearray:
    return bytearray(value.encode("cp1252") + b"\x00")


def _group_label(form_id: int) -> bytes:
    return struct.pack("<I", form_id)


def test_parse_reference_transform_reads_data_and_xscl():
    from creation_lib.worldspace_export import parse_reference_transform

    record = Record(
        "REFR",
        0x010800,
        subrecords=[
            Subrecord("DATA", struct.pack("<6f", 10.0, 20.0, 30.0, 1.0, 2.0, 3.0)),
            Subrecord("XSCL", struct.pack("<f", 1.5)),
        ],
    )

    transform = parse_reference_transform(record)

    assert transform.position == (10.0, 20.0, 30.0)
    assert transform.rotation == (1.0, 2.0, 3.0)
    assert transform.scale == 1.5


def test_model_path_prefers_modl_and_falls_back_to_mod2():
    from creation_lib.worldspace_export import extract_model_path

    with_modl = Record(
        "STAT",
        0x010801,
        subrecords=[
            Subrecord("MOD2", _zstring("Meshes\\Fallback\\Wrong.nif")),
            Subrecord("MODL", _zstring("Meshes\\Architecture\\Town\\Wall01.NIF")),
        ],
    )
    assert extract_model_path(with_modl) == "architecture/town/wall01.nif"

    modl_missing = Record(
        "ARMO",
        0x010802,
        subrecords=[Subrecord("MOD2", _zstring("Armor\\Raider\\ArmorM.nif"))],
    )
    assert extract_model_path(modl_missing) == "armor/raider/armorm.nif"


def test_resolve_mesh_path_searches_mesh_roots(tmp_path):
    from creation_lib.worldspace_export import resolve_mesh_path

    mesh_root = tmp_path / "Meshes"
    target = mesh_root / "Architecture" / "Town" / "Wall01.nif"
    target.parent.mkdir(parents=True)
    target.write_bytes(b"fake nif")

    assert resolve_mesh_path("architecture/town/wall01.nif", [mesh_root]) == target


def test_build_export_manifest_normalizes_origin_and_reports_missing_mesh(tmp_path):
    from creation_lib.worldspace_export import (
        PlacementEntry,
        PlacementTransform,
        build_export_manifest,
    )

    mesh_root = tmp_path / "Meshes"
    existing = mesh_root / "set" / "piece.nif"
    existing.parent.mkdir(parents=True)
    existing.write_bytes(b"fake nif")

    placements = [
        PlacementEntry(
            source_form_id=0x010800,
            base_form_id=0x020900,
            plugin_name="Test.esp",
            worldspace_form_id=0x000001,
            cell_form_id=0x000002,
            model_path="set/piece.nif",
            transform=PlacementTransform((10.0, 0.0, 0.0), (0.0, 0.0, 0.0), 1.0),
        ),
        PlacementEntry(
            source_form_id=0x010801,
            base_form_id=0x020901,
            plugin_name="Test.esp",
            worldspace_form_id=0x000001,
            cell_form_id=0x000003,
            model_path="set/missing.nif",
            transform=PlacementTransform((30.0, 0.0, 0.0), (0.0, 0.0, 0.0), 2.0),
        ),
    ]

    manifest = build_export_manifest(placements, [mesh_root], normalize_origin=True)

    assert manifest.origin == (20.0, 0.0, 0.0)
    assert len(manifest.placements) == 1
    assert manifest.placements[0].resolved_mesh_path == existing
    assert manifest.placements[0].transform.position == (-10.0, 0.0, 0.0)
    assert manifest.skipped[0].reason == "missing_mesh"


def test_extract_placements_resolves_base_model_in_selected_cells():
    from creation_lib.worldspace_export import extract_placements

    world_id = 0x000123
    cell_id = 0x000200
    base_id = 0x000900
    placed = Record(
        "REFR",
        0x000800,
        subrecords=[
            Subrecord("NAME", struct.pack("<I", base_id)),
            Subrecord("DATA", struct.pack("<6f", 10.0, 20.0, 30.0, 1.0, 2.0, 3.0)),
        ],
    )
    base = Record("STAT", base_id, subrecords=[Subrecord("MODL", _zstring("Set\\Piece.nif"))])
    root_items = [
        Group(
            b"WRLD",
            0,
            children=[
                Record("WRLD", world_id),
                Group(
                    _group_label(world_id),
                    1,
                    children=[
                        Record("CELL", cell_id),
                        Group(_group_label(cell_id), 6, children=[placed]),
                    ],
                ),
            ],
        )
    ]

    placements = extract_placements(
        root_items,
        plugin_name="Test.esp",
        resolve_base_record=lambda form_id: base if form_id == base_id else None,
        worldspace_form_id=world_id,
        cell_form_ids={cell_id},
    )

    assert len(placements) == 1
    assert placements[0].source_form_id == 0x000800
    assert placements[0].base_form_id == base_id
    assert placements[0].model_path == "set/piece.nif"
    assert placements[0].transform.position == (10.0, 20.0, 30.0)


def test_export_manifest_writes_sidecar_and_delegates_fbx(tmp_path):
    from creation_lib.worldspace_export import (
        PlacementEntry,
        PlacementTransform,
        ResolvedPlacement,
        ExportManifest,
        export_manifest,
    )

    mesh_path = tmp_path / "meshes" / "set" / "piece.nif"
    mesh_path.parent.mkdir(parents=True)
    mesh_path.write_bytes(b"fake nif")
    fbx_path = tmp_path / "world.fbx"
    calls = []

    def fake_fbx_exporter(manifest, output_path):
        calls.append((manifest, output_path))
        output_path.write_bytes(b"fbx")
        return output_path

    manifest = ExportManifest(
        origin=(10.0, 20.0, 30.0),
        placements=[
            ResolvedPlacement(
                PlacementEntry(
                    source_form_id=0x010800,
                    base_form_id=0x020900,
                    plugin_name="Test.esp",
                    worldspace_form_id=0x000001,
                    cell_form_id=0x000002,
                    model_path="set/piece.nif",
                    transform=PlacementTransform((10.0, 20.0, 30.0), (0.0, 0.0, 0.0), 1.0),
                ),
                mesh_path,
                PlacementTransform((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 1.0),
            )
        ],
        skipped=[],
    )

    result = export_manifest(manifest, fbx_path, fbx_exporter=fake_fbx_exporter)

    assert result.fbx_path == fbx_path
    assert result.manifest_path == fbx_path.with_suffix(".worldspace.json")
    assert calls == [(manifest, fbx_path)]
    assert '"source_form_id": "010800"' in result.manifest_path.read_text(encoding="utf-8")
