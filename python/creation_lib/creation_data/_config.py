"""Constants and helpers shared across the creation_data service layer."""

# Valid domains for the unified search tool
SEARCH_DOMAINS = {
    "wiki",
    "scripts",
    "records",
    "behaviors",
    "havok",
    "nifs",
    "ext_records",
    "ext_scripts",
}

# Type priority for records search re-ranking (lower = higher priority)
TYPE_PRIORITY = {
    "Weapons": 0, "Armors": 0, "Furniture": 0, "Npcs": 0, "Components": 0,
    "Keywords": 1, "MiscItems": 1, "Ammo": 1, "Ingestibles": 1,
    "ConstructibleObjects": 1, "ObjectModifications": 1,
    "Spells": 2, "MagicEffects": 2, "Perks": 2, "Enchantments": 2,
    "LeveledItems": 2, "LeveledNpcs": 2, "Quests": 2,
    "Activators": 3, "Containers": 3, "Globals": 3, "Factions": 3,
    "Messages": 4, "LoadScreens": 4, "Terminals": 4, "PackIns": 4,
}
DEFAULT_PRIORITY = 5

# Maps domain names to preprocessing script names for helpful error messages
DOMAIN_PREPROCESS = {
    "records": "preprocess_records",
    "scripts": "preprocess_scripts",
    "wiki": "preprocess_wiki",
    "behaviors": "preprocess_havok",
    "havok": "preprocess_havok",
    "nifs": "preprocess_nifs",
    "ext_records": "preprocess_external",
    "ext_scripts": "preprocess_external",
}

# Maps domains to their embedding table names in the database
DOMAIN_EMBEDDING_TABLE = {
    "records": "records_embeddings",
    "scripts": "scripts_embeddings",
    "wiki": "wiki_embeddings",
    "behaviors": "havok_embeddings",
    "havok": "havok_embeddings",
    "nifs": "nifs_embeddings",
    "ext_records": "ext_records_embeddings",
    "ext_scripts": "ext_scripts_embeddings",
}

# Internal fields to strip from results before returning
STRIP_FIELDS = {"editor_id_tokens", "name_tokens", "content", "yaml_path", "script_name_tokens", "script_path"}

MAX_RESULTS_LIMIT = 200
MAX_RESULTS_MIN = 1


def clamp(max_results: int) -> int:
    """Clamp max_results to a safe range."""
    if max_results < MAX_RESULTS_MIN:
        return MAX_RESULTS_MIN
    if max_results > MAX_RESULTS_LIMIT:
        return MAX_RESULTS_LIMIT
    return max_results


def strip_hit(hit: dict, extra: set = frozenset()) -> dict:
    """Remove internal/indexing fields from a result dict."""
    for f in STRIP_FIELDS | extra:
        hit.pop(f, None)
    return hit
