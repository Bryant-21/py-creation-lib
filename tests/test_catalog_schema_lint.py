"""Conventions every Tales catalog schema (creation_lib/resources/catalog_schemas) must follow.

READER_KEYS mirrors the key each Tales reader merges a section by (the lambdas handed to
MergeSection / MergedArrayBy / MergeKeyed in mods/B21_TalesFromAppalachia/src). When a reader
changes its key, change the table and the schema together.
"""

from __future__ import annotations

import itertools
import json
import re

import jsonschema
import pytest
from referencing import Registry, Resource

from creation_lib.catalog.plugin_catalog import SCHEMAS_DIR

MAP = "map"

READER_KEYS: dict[str, dict[str, list[str] | str]] = {
    "CampScrap": {"objects": MAP},
    "CampSnap": {"node_types": MAP, "templates": MAP, "forms": MAP, "fo4_aliases": MAP},
    "CampWallpapers": {"wallpapers": ["form"], "model_swaps": MAP},
    "Challenges": {"categories": ["key"], "challenges": ["id"]},
    "CombatPerks": {"cards": MAP, "items": MAP},
    "Condition": {"records": ["form"], "target_forms": MAP},
    "ContextualLoot": {
        "lists": ["id"],
        "ammo_groups": ["keyword"],
        "rewards": ["id"],
        "tokens": ["token"],
        "counts": ["list", "root", "level"],
    },
    "DailyOps": {"refs": ["form"], "entrances": ["form"], "voice": MAP},
    "Diseases": {
        "diseases": ["spell"],
        "chance_rows": ["risk", "disease"],
        "hits": ["aggressor", "aggressor_races", "disease"],
        "carriers": ["actor_type", "races"],
        "vectors": ["effect"],
        "cures": ["effect"],
        "addiction_cures": ["effect"],
        "addictions": ["spell"],
        "location_severity": ["location"],
        "triggers": ["ref"],
        "resistance_items": ["item"],
    },
    "Emotes": {"categories": ["id"], "entries": ["form"]},
    "EquipmentBonuses": {"curves": MAP},
    "Fishing": {"records": ["form"], "target_forms": MAP},
    "FoodSpoilage": {"foods": ["form"]},
    "GoldBullion": {"bullion": ["form"], "stamps": ["form"], "tadpole": ["form"], "possum": ["form"]},
    "Infestations": {"sites": ["location"], "factions": ["key"], "regions": ["location"], "mutations": ["form"]},
    "LegendaryPerks": {"bindings": MAP},
    "LeveledLists": {
        "lists": ["target"],
        "condition_forms": MAP,
        "curves": MAP,
        "regions": MAP,
        "delegates": MAP,
    },
    "NpcStats": {"curves": MAP, "npcs": MAP, "races": MAP, "weapons": MAP, "explosions": MAP, "armors": MAP},
    "ObjectiveAreas": {"areas": ["quest", "objective", "target"]},
    "PerkCardExtras": {"meat": ["form"], "survival_spells": ["form"]},
    "PerkCards": {
        "cards": ["form"],
        "excluded": ["form"],
        "weight_items": ["source_item"],
        "effect_records": MAP,
        "target_forms": MAP,
    },
    "Pets": {"pets": ["actor"], "apparel": ["form"]},
    "PlayerGhoul": {"cards": ["source"], "chems": ["form"], "appearance": ["source"]},
    "PlayerMutations": {"foods": ["form"], "consumables": ["form"], "decontamination": ["form"]},
    "QuestCategories": {"quests": ["form"]},
    "RealtimeVATS": {},
    "Reputation": {"factions": ["code"]},
    "WeaponModelSwaps": {"skins": ["form"]},
}

# Sections whose reader keeps first-appearance order (MergedArray / MergedArrayBy); the rest merge into a std::map.
ORDERED = {
    ("CampWallpapers", "wallpapers"),
    ("Challenges", "categories"),
    ("Challenges", "challenges"),
    *(("Diseases", s) for s in (
        "diseases", "chance_rows", "hits", "carriers", "vectors", "cures", "addiction_cures", "addictions",
        "location_severity", "triggers", "resistance_items",
    )),
    ("Emotes", "categories"),
    ("Emotes", "entries"),
    ("Infestations", "factions"),
    ("Infestations", "regions"),
    ("Infestations", "mutations"),
    ("PerkCardExtras", "meat"),
    ("PerkCardExtras", "survival_spells"),
    ("Pets", "pets"),
    ("Pets", "apparel"),
    ("PlayerGhoul", "cards"),
    ("PlayerGhoul", "chems"),
    ("PlayerGhoul", "appearance"),
    ("Reputation", "factions"),
}

# Sections no reader merges: ess is read whole from SeventySix.esm.json, the rest are emitter diagnostics.
OPAQUE = {
    ("LeveledLists", "ess"),
    ("LeveledLists", "diagnostics"),
    ("NpcStats", "diagnostics"),
    ("ObjectiveAreas", "skipped"),
}

WIDGETS = {"conditions", "curve"}
ENVELOPE_FORM_KEY_PATTERN = r"^[^\s:]+\.[^\s:.]+:[0-9A-Fa-f]{1,8}$"
MAP_KEY_SAMPLES = ("P.esm:000001", "B21_Legendary_Foo_1", "P-Foo", "x")
KEY_VALUE_SAMPLES = ("P.esm:000001", 1, "x", ["P.esm:000001"], None)

SCHEMA_FILES = sorted(p for p in SCHEMAS_DIR.glob("*.schema.json") if p.name != "envelope.schema.json")
FEATURES = [p.name.removesuffix(".schema.json") for p in SCHEMA_FILES]


def _load(name: str) -> dict:
    return json.loads((SCHEMAS_DIR / f"{name}.schema.json").read_text(encoding="utf-8"))


def _registry() -> Registry:
    contents = [json.loads(p.read_text(encoding="utf-8")) for p in SCHEMAS_DIR.glob("*.schema.json")]
    return Registry().with_resources([(item["$id"], Resource.from_contents(item)) for item in contents])


def _walk(node):
    yield node
    if isinstance(node, dict):
        for value in node.values():
            yield from _walk(value)
    elif isinstance(node, list):
        for value in node:
            yield from _walk(value)


def _sections(schema: dict) -> dict[str, dict]:
    return {name: body for name, body in schema["properties"].items() if name not in ("version", "settings")}


def _resolve(schema: dict, node: dict) -> dict:
    while "$ref" in node and node["$ref"].startswith("#/"):
        target: object = schema
        for part in node["$ref"][2:].split("/"):
            target = target[part]
        node = {**target, **{k: v for k, v in node.items() if k != "$ref"}}
    return node


def _entry_properties(schema: dict, section: dict) -> dict:
    section = _resolve(schema, section)
    entry = section.get("items") if section.get("type") == "array" else section.get("additionalProperties", {})
    entry = _resolve(schema, entry or {})
    props = dict(entry.get("properties", {}))
    for branch in entry.get("oneOf", []) + entry.get("allOf", []) + entry.get("anyOf", []):
        props.update(_resolve(schema, branch).get("properties", {}))
    return props


def test_table_covers_every_schema_and_all_27_features_exist_or_are_known():
    assert set(FEATURES) <= set(READER_KEYS)
    assert len(set(READER_KEYS) - set(FEATURES)) <= 1  # CampSnap's schema may still be uncommitted work in progress


@pytest.mark.parametrize("feature", FEATURES)
def test_schema_is_valid_and_follows_the_envelope(feature):
    schema = _load(feature)
    jsonschema.Draft202012Validator.check_schema(schema)
    assert schema["$id"].endswith(f"/{feature}.schema.json")
    assert {"$ref": "envelope.schema.json"} in schema["allOf"]
    assert "formKey" not in schema.get("$defs", {}), "FormKey fields $ref envelope.schema.json#/$defs/formKey directly"
    for node in _walk(schema):
        if isinstance(node, dict):
            assert node.get("$ref") != "#/$defs/formKey"
            assert node.get("pattern") != ENVELOPE_FORM_KEY_PATTERN, "duplicate of the envelope formKey pattern"


@pytest.mark.parametrize("feature", FEATURES)
def test_settings_is_a_typed_object(feature):
    settings = _load(feature)["properties"].get("settings")
    assert settings is not None, "declare settings (use an empty properties object when the feature has none)"
    assert settings.get("type") == "object"
    assert isinstance(settings.get("properties"), dict)


@pytest.mark.parametrize("feature", FEATURES)
def test_sections_match_the_reader_table(feature):
    schema = _load(feature)
    sections = _sections(schema)
    opaque = {name for name in sections if (feature, name) in OPAQUE}
    assert set(sections) - opaque == set(READER_KEYS[feature])
    assert opaque == {name for f, name in OPAQUE if f == feature}
    for name, body in sections.items():
        assert body.get("description"), f"{feature}.{name} needs a description"
        if name in opaque:
            assert body.get("x-opaque") is True and "x-key" not in body
            continue
        expected = READER_KEYS[feature][name]
        assert body.get("x-key") == expected, f"{feature}.{name} x-key"
        assert ("x-order" in body) == ((feature, name) in ORDERED), f"{feature}.{name} x-order"
        if "x-order" in body:
            assert body["x-order"] == "entries"
        resolved = _resolve(schema, body)
        assert resolved["type"] == ("object" if expected == MAP else "array"), f"{feature}.{name} type"
        if expected != MAP:
            properties = _entry_properties(schema, body)
            for field in expected:
                assert field in properties, f"{feature}.{name} key field {field} is not an entry property"


@pytest.mark.parametrize("feature", FEATURES)
def test_form_types_are_arrays_of_signatures(feature):
    for node in _walk(_load(feature)):
        if isinstance(node, dict) and "x-form-type" in node:
            value = node["x-form-type"]
            assert isinstance(value, list) and value, node
            assert all(isinstance(item, str) and re.fullmatch(r"[A-Z0-9_]{4}", item) for item in value), value


@pytest.mark.parametrize("feature", FEATURES)
def test_widgets_and_refs_are_known(feature):
    schema = _load(feature)
    sections = _sections(schema)
    for node in _walk(schema):
        if not isinstance(node, dict):
            continue
        if "x-widget" in node:
            assert node["x-widget"] in WIDGETS, node["x-widget"]
        if "x-ref" in node:
            target = node["x-ref"]
            assert target in sections, f"x-ref {target} is not a section of {feature}"
            key = sections[target].get("x-key")
            assert key == MAP or (isinstance(key, list) and len(key) == 1), f"x-ref {target} needs a single-field key"


@pytest.mark.parametrize("feature", FEATURES)
def test_every_entry_type_accepts_remove(feature):
    schema = _load(feature)
    registry = _registry()
    schema_without_required = {k: v for k, v in schema.items() if k != "required"}
    validator = jsonschema.Draft202012Validator(schema_without_required, registry=registry)
    for name, body in _sections(schema).items():
        if (feature, name) in OPAQUE:
            continue
        if READER_KEYS[feature][name] == MAP:
            candidates = [{"version": 1, name: {key: {"remove": True}}} for key in MAP_KEY_SAMPLES]
        else:
            fields = READER_KEYS[feature][name]
            rows = [
                {"remove": True, **dict(zip(fields, values))}
                for values in itertools.product(KEY_VALUE_SAMPLES, repeat=len(fields))
            ]
            candidates = [{"version": 1, name: [row]} for row in rows]
        assert any(validator.is_valid(document) for document in candidates), f"{feature}.{name} rejects {{\"remove\": true}}"


ASSET_KINDS = {"nif", "texture", "material", "swf", "sound", "any"}
ASSET_ROOTS = {"Meshes", "Textures", "Materials", "Interface", "Sound", ""}


@pytest.mark.parametrize("feature", FEATURES)
def test_editor_annotations_are_well_formed(feature):
    """x-asset / x-enum-labels / x-flags / x-columns / x-key-title / x-key-form-type / x-help shapes."""
    schema = _load(feature)
    for node in _walk(schema):
        if not isinstance(node, dict):
            continue
        if "x-asset" in node:
            asset = node["x-asset"]
            assert set(asset) == {"kind", "root", "strip_root"}, asset
            assert asset["kind"] in ASSET_KINDS and asset["root"] in ASSET_ROOTS, asset
            assert isinstance(asset["strip_root"], bool), asset
        if "x-flags" in node:
            assert node.get("type") == "integer", node
            assert all(key.isdecimal() and isinstance(label, str) for key, label in node["x-flags"].items())
        if "x-enum-labels" in node:
            assert all(isinstance(key, str) and isinstance(label, str) for key, label in node["x-enum-labels"].items())
        if "x-columns" in node:
            assert isinstance(node["x-columns"], list) and all(isinstance(c, str) for c in node["x-columns"])
        for key in ("x-key-title", "x-help"):
            if key in node:
                assert isinstance(node[key], str) and node[key].strip()
        if "x-key-form-type" in node:
            value = node["x-key-form-type"]
            assert isinstance(value, list) and all(re.fullmatch(r"[A-Z0-9_]{4}", item) for item in value), value
