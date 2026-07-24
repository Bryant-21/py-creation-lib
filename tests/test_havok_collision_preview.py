from __future__ import annotations


def test_blob_preview_delegates_to_native(monkeypatch) -> None:
    from creation_lib.havok import native_runtime
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    calls: list[tuple[bytes, float, int | None]] = []

    def collision_preview_native(
        blob: bytes,
        havok_scale: float = 1.0,
        body_id: int | None = None,
    ) -> dict:
        calls.append((blob, havok_scale, body_id))
        return {
            "meshes": [
                {
                    "shape_type": "sphere",
                    "mesh": {
                        "vertices": [{"x": 0.0, "y": 0.0, "z": 0.0}],
                        "triangles": [],
                    },
                }
            ]
        }

    monkeypatch.setattr(native_runtime, "collision_preview_native", collision_preview_native)

    previews = extract_preview_meshes_from_blob(b"blob", havok_scale=70.0, body_id=2)

    assert calls == [(b"blob", 70.0, 2)]
    assert previews[0]["shape_type"] == "sphere"


def test_blob_preview_returns_empty_list_when_native_has_no_meshes(monkeypatch) -> None:
    from creation_lib.havok import native_runtime
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    monkeypatch.setattr(
        native_runtime,
        "collision_preview_native",
        lambda _blob, havok_scale=1.0, body_id=None: {"meshes": []},
    )

    assert extract_preview_meshes_from_blob(b"blob", havok_scale=1.0) == []


def test_np_collision_overlay_keeps_individual_shape_meshes(monkeypatch) -> None:
    from creation_lib.renderer import nif_loader

    class Block:
        def __init__(self, block_id, type_name, fields):
            self.block_id = block_id
            self.type_name = type_name
            self.fields = fields

        def get_field(self, name):
            return self.fields.get(name)

    physics = Block(31, "bhkPhysicsSystem", {"Binary Data": {"Data": [1, 2]}})
    collision = Block(30, "bhkNPCollisionObject", {"Data": 31, "Body ID": 4})
    nif = type(
        "Nif",
        (),
        {"get_block": lambda _self, block_id: physics if block_id == 31 else None},
    )()
    monkeypatch.setattr(
        nif_loader,
        "extract_preview_meshes_from_blob",
        lambda _blob, havok_scale, body_id: [
            {
                "shape_type": "convex_hull",
                "mesh": {
                    "vertices": [
                        {"x": 0.0, "y": 0.0, "z": 0.0},
                        {"x": 1.0, "y": 0.0, "z": 0.0},
                        {"x": 0.0, "y": 1.0, "z": 0.0},
                    ],
                    "triangles": [{"v1": 0, "v2": 1, "v3": 2}],
                },
            },
            {
                "shape_type": "compressed_mesh",
                "mesh": {
                    "vertices": [
                        {"x": 0.0, "y": 0.0, "z": 1.0},
                        {"x": 1.0, "y": 0.0, "z": 1.0},
                        {"x": 0.0, "y": 1.0, "z": 1.0},
                    ],
                    "triangles": [{"v1": 0, "v2": 1, "v3": 2}],
                },
            },
        ],
    )

    shapes = nif_loader._extract_np_collision_shapes(nif, collision, havok_scale=70.0)

    assert [shape.shape_type for shape in shapes] == [
        "convex_hull",
        "compressed_mesh",
    ]
    assert [shape.shape_index for shape in shapes] == [0, 1]
    assert all(shape.source_block_id == 31 for shape in shapes)
    assert all(shape.body_id == 4 for shape in shapes)
    assert all(shape.triangles.shape == (1, 3) for shape in shapes)
    assert all(shape.positions.shape == (6, 3) for shape in shapes)


def test_legacy_list_collision_keeps_individual_shape_block_ids() -> None:
    from creation_lib.renderer import nif_loader

    class Block:
        def __init__(self, block_id, type_name, fields):
            self.block_id = block_id
            self.type_name = type_name
            self.fields = fields

        def get_field(self, name):
            return self.fields.get(name)

    blocks = {
        1: Block(1, "bhkCollisionObject", {"Body": 2}),
        2: Block(2, "bhkRigidBody", {"Shape": 3}),
        3: Block(3, "bhkListShape", {"Sub Shapes": [4, 5]}),
        4: Block(
            4,
            "bhkBoxShape",
            {"Dimensions": {"x": 1.0, "y": 2.0, "z": 3.0}},
        ),
        5: Block(5, "bhkSphereShape", {"Radius": 2.0}),
    }
    nif = type("Nif", (), {"get_block": lambda _self, block_id: blocks.get(block_id)})()

    shapes = nif_loader._extract_legacy_collision_shapes(
        nif, blocks[1], havok_scale=1.0
    )

    assert [shape.source_block_id for shape in shapes] == [4, 5]
    assert [shape.shape_type for shape in shapes] == ["bhkBoxShape", "bhkSphereShape"]
    assert all(len(shape.vertices) > 0 for shape in shapes)
    assert all(len(shape.triangles) > 0 for shape in shapes)
