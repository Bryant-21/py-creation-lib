"""Preview extraction for Gamebryo-era (FO3/FNV/Oblivion) collision shapes."""

import numpy as np
import pytest


class _Block:
    def __init__(self, block_id, type_name, fields=None):
        self.block_id = block_id
        self.type_name = type_name
        self._fields = fields or {}

    def get_field(self, name):
        return self._fields.get(name)


class _Nif:
    def __init__(self, blocks):
        self._blocks = {block.block_id: block for block in blocks}

    def get_block(self, block_id):
        return self._blocks.get(block_id)


def _triangle(v1, v2, v3):
    return {"Triangle": {"v1": v1, "v2": v2, "v3": v3}}


def _packed_strips_nif(body_type="bhkRigidBody", translation=None):
    """A MOPP-wrapped packed tri-strips tetrahedron, as FNV rocks and buildings use."""
    info = {
        "Rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
        "Translation": translation or {"x": 0.0, "y": 0.0, "z": 0.0},
    }
    return _Nif(
        [
            _Block(0, "NiNode", {"Collision Object": 1}),
            _Block(1, "bhkCollisionObject", {"Target": 0, "Body": 2}),
            _Block(2, body_type, {"Shape": 3, "Rigid Body Info:550_660": info}),
            _Block(3, "bhkMoppBvTreeShape", {"Shape": 4}),
            _Block(
                4,
                "bhkPackedNiTriStripsShape",
                {"Data": 5, "Scale": {"x": 1.0, "y": 1.0, "z": 1.0, "w": 0.0}},
            ),
            _Block(
                5,
                "hkPackedNiTriStripsData",
                {
                    "Compressed": 0,
                    "Vertices": [
                        {"x": 0.0, "y": 0.0, "z": 0.0},
                        {"x": 10.0, "y": 0.0, "z": 0.0},
                        {"x": 0.0, "y": 10.0, "z": 0.0},
                        {"x": 0.0, "y": 0.0, "z": 10.0},
                    ],
                    "Triangles": [
                        _triangle(0, 1, 2),
                        _triangle(0, 1, 3),
                        _triangle(0, 2, 3),
                        _triangle(1, 2, 3),
                    ],
                },
            ),
        ]
    )


def test_packed_tri_strips_shape_produces_preview_geometry():
    """Regression: FNV/FO3 mesh collision rendered nothing in the NIF editor."""
    from creation_lib.renderer.nif_loader import _extract_legacy_collision_shapes

    nif = _packed_strips_nif()
    shapes = _extract_legacy_collision_shapes(
        nif, nif.get_block(1), havok_scale=6.999125
    )

    assert len(shapes) == 1
    assert shapes[0].shape_type == "bhkPackedNiTriStripsShape"
    assert len(shapes[0].vertices) == 4
    assert len(shapes[0].triangles) == 4
    assert len(shapes[0].positions) > 0


def test_packed_tri_strips_shape_uses_legacy_havok_scale():
    """10 legacy Havok units is ~70 game units, not ~700."""
    from creation_lib.renderer.nif_loader import _extract_legacy_collision_shapes

    nif = _packed_strips_nif()
    shapes = _extract_legacy_collision_shapes(
        nif, nif.get_block(1), havok_scale=6.999125
    )

    assert shapes[0].vertices.max() == pytest.approx(69.99125, rel=1e-4)


def test_rigid_body_t_translation_is_baked_into_the_preview():
    """bhkRigidBodyT bakes its transform into the shape; bhkRigidBody does not."""
    from creation_lib.renderer.nif_loader import _extract_legacy_collision_shapes

    translation = {"x": 0.0, "y": 0.0, "z": -10.0}
    plain = _packed_strips_nif("bhkRigidBody", translation)
    transformed = _packed_strips_nif("bhkRigidBodyT", translation)

    plain_shapes = _extract_legacy_collision_shapes(
        plain, plain.get_block(1), havok_scale=6.999125
    )
    transformed_shapes = _extract_legacy_collision_shapes(
        transformed, transformed.get_block(1), havok_scale=6.999125
    )

    assert plain_shapes[0].vertices[:, 2].min() == pytest.approx(0.0, abs=1e-4)
    assert transformed_shapes[0].vertices[:, 2].min() == pytest.approx(-69.99125, rel=1e-4)


def test_compressed_packed_vertices_are_skipped_rather_than_misread():
    from creation_lib.renderer.nif_loader import _extract_legacy_collision_shapes

    nif = _packed_strips_nif()
    nif.get_block(5)._fields["Compressed"] = 1

    assert _extract_legacy_collision_shapes(nif, nif.get_block(1)) == []


def test_legacy_games_declare_the_gamebryo_havok_scale():
    """FNV/FO3/Oblivion collision is authored at a tenth of the FO4 Havok scale."""
    from creation_lib.core.game_profiles import get_profile

    for game in ("fnv", "fo3", "oblivion"):
        assert get_profile(game).havok_scale == pytest.approx(6.999125), game
    assert get_profile("fo4").havok_scale == pytest.approx(69.99125)
