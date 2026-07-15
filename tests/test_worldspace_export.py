import struct

from creation_lib.esp.model import Group, PluginHeader, Record, Subrecord


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


def test_model_path_prefers_modl_and_normalizes_data_relative_paths():
    from creation_lib.worldspace_export import extract_model_path

    record = Record(
        "STAT",
        0x010801,
        subrecords=[
            Subrecord("MOD2", _zstring("Meshes\\Fallback\\Wrong.nif")),
            Subrecord("MODL", _zstring("Meshes\\Architecture\\Town\\Wall01.NIF")),
        ],
    )

    assert extract_model_path(record) == "architecture/town/wall01.nif"


def test_model_path_uses_armor_world_model_when_modl_is_missing():
    from creation_lib.worldspace_export import extract_model_path

    record = Record(
        "ARMO",
        0x010802,
        subrecords=[Subrecord("MOD2", _zstring("Armor\\Raider\\ArmorM.nif"))],
    )

    assert extract_model_path(record) == "armor/raider/armorm.nif"


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


def test_list_worldspaces_reads_editor_id_and_name_from_records():
    from creation_lib.worldspace_export import list_worldspaces

    root_items = [
        Group(
            b"WRLD",
            0,
            children=[
                Record(
                    "WRLD",
                    0x000123,
                    subrecords=[
                        Subrecord("EDID", _zstring("B21_TestWorld")),
                        Subrecord("FULL", _zstring("Test World")),
                    ],
                )
            ],
        )
    ]

    worldspaces = list_worldspaces(root_items)

    assert [(w.form_id, w.editor_id, w.name) for w in worldspaces] == [
        (0x000123, "B21_TestWorld", "Test World")
    ]


def test_list_cells_scopes_cells_to_worldspace_group():
    from creation_lib.worldspace_export import list_cells

    world_id = 0x000123
    matching_cell = Record(
        "CELL",
        0x000200,
        subrecords=[Subrecord("EDID", _zstring("B21_TestWorld_Cell"))],
    )
    other_cell = Record(
        "CELL",
        0x000201,
        subrecords=[Subrecord("EDID", _zstring("OtherWorld_Cell"))],
    )
    root_items = [
        Group(
            b"WRLD",
            0,
            children=[
                Record("WRLD", world_id),
                Group(_group_label(world_id), 1, children=[matching_cell]),
                Record("WRLD", 0x000124),
                Group(_group_label(0x000124), 1, children=[other_cell]),
            ],
        )
    ]

    cells = list_cells(root_items, world_id)

    assert [(c.form_id, c.editor_id, c.worldspace_form_id) for c in cells] == [
        (0x000200, "B21_TestWorld_Cell", world_id)
    ]


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


def test_loaded_bundle_resolves_base_records_through_active_plugin_masters():
    from creation_lib.esp import Plugin
    from creation_lib.worldspace_export import LoadedPluginBundle

    world_id = 0x000123
    cell_id = 0x000200
    base_object_id = 0x000900
    placed = Record(
        "REFR",
        0x000800,
        subrecords=[
            Subrecord("NAME", struct.pack("<I", base_object_id)),
            Subrecord("DATA", struct.pack("<6f", 1.0, 2.0, 3.0, 0.0, 0.0, 0.0)),
        ],
    )
    active = Plugin(
        plugin_name="Patch.esp",
        header=PluginHeader(masters=["Base.esm"]),
        root_items=[
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
        ],
    )
    master = Plugin(
        plugin_name="Base.esm",
        header=PluginHeader(),
        root_items=[
            Group(
                b"STAT",
                0,
                children=[
                    Record(
                        "STAT",
                        base_object_id,
                        subrecords=[Subrecord("MODL", _zstring("Set\\Piece.nif"))],
                    )
                ],
            )
        ],
    )

    bundle = LoadedPluginBundle(active_plugin=active, plugins=[master, active])

    placements = bundle.extract_placements(world_id, {cell_id})

    assert len(placements) == 1
    assert placements[0].base_form_id == base_object_id
    assert placements[0].model_path == "set/piece.nif"


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
