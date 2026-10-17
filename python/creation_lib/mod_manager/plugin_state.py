from dataclasses import asdict, dataclass
import json
from .backend import call


@dataclass(frozen=True)
class Plugin:
    name: str
    path: str
    provider: str
    mod_id: str
    enabled: bool
    priority: int
    forced: bool = False
    header: dict | None = None
    masters: tuple[str, ...] | None = None
    fingerprint: tuple[int, int] | None = None
    ghosted: bool = False
    library_priority: int = 0


@dataclass(frozen=True)
class Catalog:
    profile_id: str
    signature: str
    game: str
    plugins: tuple[Plugin, ...]
    issues: tuple[str, ...] = ()
    locked: tuple[tuple[str, int], ...] = ()
    custom_order: bool = True


@dataclass(frozen=True)
class Review:
    catalog: Catalog
    order: tuple[str, ...]
    enabled: frozenset[str]
    locked: tuple[tuple[str, int], ...] = ()


DOMAINS = {"fallout4": "fo4", "skyrimspecialedition": "skyrimse", "skyrim": "skyrim", "fallout3": "fo3", "newvegas": "fnv", "starfield": "starfield"}
BASE = {"fo4": "Fallout4.esm", "skyrimse": "Skyrim.esm", "skyrim": "Skyrim.esm", "fo3": "Fallout3.esm", "fnv": "FalloutNV.esm", "starfield": "Starfield.esm"}
FO4_PRIMARY = ("Fallout4.esm", "DLCRobot.esm", "DLCworkshop01.esm", "DLCCoast.esm", "DLCworkshop02.esm", "DLCworkshop03.esm", "DLCNukaWorld.esm", "DLCUltraHighResolution.esm")



def _catalog_json(catalog):
    return json.dumps({"profile_id": catalog.profile_id, "signature": catalog.signature,
                       "game": catalog.game, "plugins": [asdict(p) for p in catalog.plugins],
                       "issues": catalog.issues, "locked": catalog.locked, "custom_order": catalog.custom_order})


def _catalog(data):
    return Catalog(data["profile_id"], data["signature"], data["game"],
                   tuple(Plugin(**{**p, "fingerprint": tuple(p["fingerprint"]) if p["fingerprint"] is not None else None,
                                   "masters": tuple(p["masters"]) if p["masters"] is not None else None}) for p in data["plugins"]),
                   tuple(data["issues"]), tuple(tuple(v) for v in data["locked"]), data.get("custom_order", True))


def default_order(catalog):
    return call("plugin_state_default_order", _catalog_json(catalog))


def is_master(plugin):
    return call("plugin_state_flag", json.dumps(asdict(plugin)), False)


def is_light(plugin):
    return call("plugin_state_flag", json.dumps(asdict(plugin)), True)


def plugin_name(value):
    return call("plugin_state_name", value)


def signature(library, profile_id):
    return call("plugin_inventory_signature", library, profile_id)


def database_signature_for(library, db, profile_id):
    return call("plugin_inventory_signature_for", library, db, profile_id)


def database_signature(db, profile_id):
    return call("library_signature", db.native, profile_id)


def snapshot(files, name):
    return call("plugin_state_snapshot", json.dumps(files), name)


def lines(text, *, stars=False):
    return call("plugin_state_lines", text, stars)


def locked_order(text):
    return tuple(call("plugin_state_locks", text))


def load_indices(catalog, order, enabled):
    return call("plugin_state_indices", _catalog_json(catalog), list(order), list(enabled))


def provider_files(root, cancel):
    return iter(call("plugin_provider_files", root, cancel))


def inventory(library, profile_id, cancel=None, progress=None):
    return _catalog(json.loads(call("plugin_inventory", library, profile_id, cancel, progress)))


def modkit_metadata(path, game, cancel=None):
    return call("plugin_metadata", path, game, cancel)


class CachedMetadata:
    def __init__(self, library, reader=None):
        self.library, self.reader = library, reader or modkit_metadata

    def __call__(self, path, game, cancel=None):
        return call("plugin_cached_metadata", self.library, self.reader, path, game, cancel)


def inspect(catalog, reader=modkit_metadata, cancel=None, progress=None):
    return call("plugin_inspect", catalog, reader, cancel, progress)


def review(catalog, order=None, enabled=None, locked=None):
    result = json.loads(call("plugin_state_review", _catalog_json(catalog), json.dumps(list(order) if order is not None else None),
                             json.dumps(list(enabled) if enabled is not None else None), json.dumps(list(locked) if locked is not None else None)))
    return Review(catalog, tuple(result["order"]), frozenset(result["enabled"]), tuple(tuple(v) for v in result["locked"]))


def apply(library, plan, cancel=None):
    return call("plugin_apply", library, plan, cancel)


def validate_runtime(files, winners, game_root, overwrite, original_game_root=""):
    return call("plugin_validate_runtime", json.dumps(files), winners, game_root, overwrite, original_game_root)
