"""Public schema API.

get_schema(game) returns a GameSchema composed from common records +
per-game overrides. Per-game module always wins on conflict.

Legacy corpus/manifest helpers are also re-exported here (used by the
drift detector and corpus tests).
"""
from __future__ import annotations
import importlib
from functools import lru_cache
from pathlib import Path

from .base import (
    ArraySpec,
    ConditionSpec,
    EnumDef,
    FieldSpec,
    GameSchema,
    RecordFlagBit,
    RecordFlagsSpec,
    RecordSpec,
    SubrecordSpec,
    TargetMapEntry,
    UnionVariantSpec,
)
from .kinds import FieldKind
from creation_lib.esp.schema.corpus import (
    OFFICIAL_PLUGIN_ALLOWLISTS,
    build_corpus_manifest,
    build_layered_manifest,
    get_official_allowlist,
    iter_official_plugin_paths,
    record_coverage_rows,
    schema_coverage_summary,
)
from creation_lib.esp.schema.manifest import (
    RecordMember,
    RecordObservation,
    SchemaManifest,
    SubrecordObservation,
    XEditRecordSchema,
)
from creation_lib.esp.schema.semantic_overlay import (
    FieldSemanticOverlay,
    RecordSemanticOverlay,
    SemanticOverlay,
    SubrecordSemanticOverlay,
    apply_semantic_overlay,
    build_semantic_overlays,
    collect_overlay_changes,
    default_overlay_dir,
    default_overlay_path,
    install_semantic_overlay,
    load_installed_overlay,
)

SUPPORTED_GAMES: tuple[str, ...] = (
    "fo4", "starfield", "fo76", "skyrimse", "oblivion", "fnv", "fo3",
)


def _merge_schema_chain(parent: GameSchema, child: GameSchema) -> GameSchema:
    """Combine parent/child schemas with child definitions winning."""
    return GameSchema(
        game=child.game,
        records={**parent.records, **child.records},
        header_version=child.header_version,
        localized_support=child.localized_support,
        enums={**parent.enums, **child.enums},
    )


def _load_schema_chain(game: str, ancestry: tuple[str, ...] = ()) -> GameSchema:
    if game in ancestry:
        chain = " -> ".join((*ancestry, game))
        raise ValueError(f"Schema inheritance cycle detected: {chain}")

    mod = importlib.import_module(f".games.{game}", __package__)
    schema = mod.build_schema()
    parent_game = getattr(mod, "EXTENDS", None)
    if not parent_game:
        return schema

    parent_schema = _load_schema_chain(parent_game, (*ancestry, game))
    return _merge_schema_chain(parent_schema, schema)


@lru_cache(maxsize=None)
def _get_schema_cached(game: str, apply_overlays: bool, overlay_dir_str: str | None) -> GameSchema:
    schema = _load_schema_chain(game)
    if not apply_overlays:
        return schema
    overlay = load_installed_overlay(game, overlay_dir=overlay_dir_str)
    if overlay is None:
        return schema
    return apply_semantic_overlay(schema, overlay)


def get_schema(
    game: str,
    *,
    apply_overlays: bool = True,
    overlay_dir: str | Path | None = None,
) -> GameSchema:
    """Load and return the GameSchema for `game`.

    Schemas are cached indefinitely per `(game, apply_overlays, overlay_dir)`
    tuple — schema definitions are static at runtime. Call
    ``_get_schema_cached.cache_clear()`` to invalidate during tests if needed.

    Raises ValueError if `game` is not in SUPPORTED_GAMES.
    """
    if game not in SUPPORTED_GAMES:
        raise ValueError(
            f"Unknown game {game!r}; supported: {SUPPORTED_GAMES}"
        )
    overlay_key = None if overlay_dir is None else str(overlay_dir)
    return _get_schema_cached(game, apply_overlays, overlay_key)


__all__ = [
    "FieldKind",
    "GameSchema",
    "OFFICIAL_PLUGIN_ALLOWLISTS",
    "RecordFlagBit",
    "RecordFlagsSpec",
    "RecordMember",
    "RecordObservation",
    "RecordSpec",
    "RecordSemanticOverlay",
    "SchemaManifest",
    "SemanticOverlay",
    "SUPPORTED_GAMES",
    "SubrecordSemanticOverlay",
    "SubrecordObservation",
    "SubrecordSpec",
    "XEditRecordSchema",
    "build_corpus_manifest",
    "build_layered_manifest",
    "build_semantic_overlays",
    "ArraySpec",
    "ConditionSpec",
    "collect_overlay_changes",
    "default_overlay_dir",
    "default_overlay_path",
    "EnumDef",
    "FieldSemanticOverlay",
    "FieldSpec",
    "get_official_allowlist",
    "get_schema",
    "install_semantic_overlay",
    "iter_official_plugin_paths",
    "load_installed_overlay",
    "record_coverage_rows",
    "schema_coverage_summary",
    "apply_semantic_overlay",
    "TargetMapEntry",
    "UnionVariantSpec",
]
