"""Tests for NIF import categorization and merge logic."""
from __future__ import annotations

import pytest
from unittest.mock import MagicMock


def _make_nif_with_children(child_types: list[str]):
    """Create a mock NifFile with root (block 0) and typed children."""
    nif = MagicMock()
    schema = MagicMock()
    nif.schema = schema
    # is_subtype_of: return True only for exact match (sufficient for tests)
    schema.is_subtype_of = lambda t, base: t == base

    root = MagicMock()
    root.block_id = 0
    root.type_name = "BSFadeNode"
    # Children field: list of block IDs [1, 2, 3, ...]
    root.get_field = lambda name: list(range(1, len(child_types) + 1)) if name == "Children" else None

    blocks = [root]
    for i, tname in enumerate(child_types):
        child = MagicMock()
        child.block_id = i + 1
        child.type_name = tname
        blocks.append(child)

    nif.blocks = blocks
    nif.get_block = lambda bid: blocks[bid] if 0 <= bid < len(blocks) else None
    return nif


class TestCategorizeChildren:
    def test_empty_root(self):
        """Root with no children returns empty categories."""
        from creation_lib.renderer.nif_importer import categorize_children

        nif = _make_nif_with_children([])
        nif.get_block(0).get_field = lambda name: [] if name == "Children" else None
        result = categorize_children(nif, 0)
        assert result == {
            "geometry": [],
            "animations": [],
            "connect_points": [],
            "root_extra_data": [],
        }

    def test_mixed_children(self):
        """Children are sorted into correct categories."""
        from creation_lib.renderer.nif_importer import categorize_children

        nif = _make_nif_with_children([
            "BSTriShape",                          # geometry (1)
            "NiControllerManager",                 # animation (2)
            "BSConnectPoint::Parents",             # connect point (3)
            "BSXFlags",                            # root extra data (4)
            "NiNode",                              # geometry (5)
            "NiControllerSequence",                # animation (6)
            "BSConnectPoint::Children",            # connect point (7)
            "BSBehaviorGraphExtraData",            # root extra data (8)
            "NiMultiTargetTransformController",    # animation (9)
            "NiDefaultAVObjectPalette",            # root extra data (10)
        ])
        result = categorize_children(nif, 0)
        assert result["geometry"] == [1, 5]
        assert result["animations"] == [2, 6, 9]
        assert result["connect_points"] == [3, 7]
        assert result["root_extra_data"] == [4, 8, 10]

    def test_all_geometry(self):
        """Unknown types default to geometry bucket."""
        from creation_lib.renderer.nif_importer import categorize_children

        nif = _make_nif_with_children(["BSTriShape", "NiPointLight", "BSEffectShaderProperty"])
        result = categorize_children(nif, 0)
        assert result["geometry"] == [1, 2, 3]
        assert result["animations"] == []
        assert result["connect_points"] == []
        assert result["root_extra_data"] == []


def _make_real_nif_pair():
    """Create two minimal real NifFile instances for import testing.

    Source has root (BSFadeNode) with:
      - block 1: BSTriShape (geometry)
      - block 2: BSXFlags (root extra data)
    Target has just a root (BSFadeNode).
    """
    from creation_lib.nif.nif_file import NifFile

    source = NifFile.new("FO4")
    # Add a BSTriShape child
    shape = source.add_block("BSTriShape", {"Name": "ImportedShape"})
    shape_id = shape.block_id
    # Attach to root children
    root = source.blocks[0]
    children = root.get_field("Children") or []
    children.append(shape_id)
    root.set_field("Children", children)
    root.set_field("Num Children", len(children))

    # Add BSXFlags as a child of root
    flags = source.add_block("BSXFlags", {"Name": "BSX", "Integer Data": 0})
    flags_id = flags.block_id
    children = root.get_field("Children") or []
    children.append(flags_id)
    root.set_field("Children", children)
    root.set_field("Num Children", len(children))

    target = NifFile.new("FO4")
    return source, target


class TestImportNif:
    def test_import_all(self):
        """Import with all options enabled merges all blocks."""
        from creation_lib.renderer.nif_importer import import_nif, ImportOptions

        source, target = _make_real_nif_pair()
        target_blocks_before = len(target.blocks)

        app = MagicMock()
        app.nif = target
        app.nif_file = target

        opts = ImportOptions()  # all True
        result = import_nif(app, source, opts)

        assert result.error == ""
        assert result.imported_count > 0
        assert len(target.blocks) > target_blocks_before

    def test_import_geometry_only(self):
        """With only geometry enabled, animations/connect_points/extra_data skipped."""
        from creation_lib.renderer.nif_importer import import_nif, ImportOptions

        source, target = _make_real_nif_pair()

        app = MagicMock()
        app.nif = target
        app.nif_file = target

        opts = ImportOptions(
            import_geometry=True,
            import_animations=False,
            import_connect_points=False,
            import_root_extra_data=False,
        )
        result = import_nif(app, source, opts)
        assert result.error == ""
        # BSXFlags should NOT have been imported
        type_names = [b.type_name for b in target.blocks]
        assert "BSXFlags" not in type_names

    def test_import_nothing(self):
        """All options disabled imports zero blocks."""
        from creation_lib.renderer.nif_importer import import_nif, ImportOptions

        source, target = _make_real_nif_pair()
        target_blocks_before = len(target.blocks)

        app = MagicMock()
        app.nif = target
        app.nif_file = target

        opts = ImportOptions(
            import_geometry=False,
            import_animations=False,
            import_connect_points=False,
            import_root_extra_data=False,
        )
        result = import_nif(app, source, opts)
        assert result.imported_count == 0
        assert len(target.blocks) == target_blocks_before

    def test_duplicate_extra_data_skipped(self):
        """If target root already has BSXFlags, importing BSXFlags is skipped."""
        from creation_lib.renderer.nif_importer import import_nif, ImportOptions

        source, target = _make_real_nif_pair()
        # Add BSXFlags to target root too
        t_flags = target.add_block("BSXFlags", {"Name": "BSX", "Integer Data": 0})
        t_root = target.blocks[0]
        extra = t_root.get_field("Extra Data List") or []
        extra.append(t_flags.block_id)
        t_root.set_field("Extra Data List", extra)
        t_root.set_field("Num Extra Data List", len(extra))

        app = MagicMock()
        app.nif = target
        app.nif_file = target

        opts = ImportOptions(import_geometry=False, import_root_extra_data=True)
        result = import_nif(app, source, opts)
        assert any("BSXFlags" in s for s in result.skipped)

    def test_import_with_no_nif_returns_error(self):
        """import_nif returns error when app has no NIF loaded."""
        from creation_lib.renderer.nif_importer import import_nif, ImportOptions

        source, _ = _make_real_nif_pair()
        app = MagicMock()
        app.nif = None
        app.nif_file = None

        result = import_nif(app, source, ImportOptions())
        assert result.error != ""


class TestImportOptions:
    def test_to_dict_defaults(self):
        from creation_lib.renderer.nif_importer import ImportOptions
        opts = ImportOptions()
        d = opts.to_dict()
        assert d == {
            "import_geometry": True,
            "import_animations": True,
            "import_connect_points": True,
            "import_root_extra_data": True,
        }

    def test_from_dict_round_trip(self):
        from creation_lib.renderer.nif_importer import ImportOptions
        original = ImportOptions(import_geometry=False, import_animations=True,
                                  import_connect_points=False, import_root_extra_data=True)
        d = original.to_dict()
        restored = ImportOptions.from_dict(d)
        assert restored == original

    def test_from_dict_missing_keys(self):
        from creation_lib.renderer.nif_importer import ImportOptions
        opts = ImportOptions.from_dict({})
        assert opts == ImportOptions()  # all defaults True
